/**
 * The picture: three.js, one instanced mesh for the nodes, one line buffer for the edges.
 *
 * The Definition of Done for M8 asks that five thousand nodes stay interactive, and that number
 * is what dictates the shape of this file. Every node is one instance of one geometry, every
 * edge is two vertices in one buffer, and both are uploaded once. Per-node meshes would be
 * five thousand draw calls; per-edge objects would be twenty thousand. Neither survives a
 * mouse drag.
 *
 * The two things that are allowed to be expensive are picking, which happens on a click, and
 * the layout transition, which rewrites the edge buffer for the second and a half it lasts.
 */

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { EffectComposer } from "three/examples/jsm/postprocessing/EffectComposer.js";
import { RenderPass } from "three/examples/jsm/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "three/examples/jsm/postprocessing/UnrealBloomPass.js";
import { EDGE_FALLBACK, EDGE_STYLE, BACKGROUND, nodeColor, nodeRadius, toHex } from "./palette";
import type { GraphEdge, GraphNode, GraphSnapshot } from "./snapshot";
import { layout, type Positions } from "./layout";

export type LayoutMode = "tree" | "shells";

const TRANSITION_MS = 1600;
/** Above this, the halo layer is dropped before the node layer: glow is the cheaper loss. */
const HALO_BUDGET = 20_000;

function easeInOut(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

/** A soft dot, drawn once into a texture rather than computed per fragment. */
function haloTexture(): THREE.Texture {
  const size = 64;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d")!;
  const gradient = ctx.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
  gradient.addColorStop(0, "rgba(255,255,255,1)");
  gradient.addColorStop(0.35, "rgba(255,255,255,0.35)");
  gradient.addColorStop(1, "rgba(255,255,255,0)");
  ctx.fillStyle = gradient;
  ctx.fillRect(0, 0, size, size);
  const texture = new THREE.CanvasTexture(canvas);
  texture.needsUpdate = true;
  return texture;
}

const HALO_VERTEX = /* glsl */ `
  attribute float size;
  attribute vec3 haloColor;
  attribute float alpha;
  varying vec3 vColor;
  varying float vAlpha;
  void main() {
    vColor = haloColor;
    vAlpha = alpha;
    vec4 mv = modelViewMatrix * vec4(position, 1.0);
    gl_PointSize = size * (300.0 / -mv.z);
    gl_Position = projectionMatrix * mv;
  }
`;

const HALO_FRAGMENT = /* glsl */ `
  uniform sampler2D map;
  varying vec3 vColor;
  varying float vAlpha;
  void main() {
    vec4 texel = texture2D(map, gl_PointCoord);
    gl_FragColor = vec4(vColor, texel.a * vAlpha);
    if (gl_FragColor.a < 0.01) discard;
  }
`;

/** A ring that grows and fades — one episode, happening. */
const PULSE_VERTEX = /* glsl */ `
  attribute float born;
  attribute float speed;
  attribute vec3 pulseColor;
  uniform float now;
  varying vec3 vColor;
  varying float vAlpha;
  void main() {
    float age = (now - born) * speed;
    vColor = pulseColor;
    vAlpha = clamp(1.0 - age, 0.0, 1.0);
    vec4 mv = modelViewMatrix * vec4(position, 1.0);
    gl_PointSize = (6.0 + age * 90.0) * (300.0 / -mv.z);
    gl_Position = projectionMatrix * mv;
  }
`;

export interface SceneOptions {
  container: HTMLElement;
  snapshot: GraphSnapshot;
  /**
   * Draw the glow pass. On by default, because it is what the standalone view looks like.
   *
   * Off inside the Agent Inspector: that window runs on WebKitGTK with the dmabuf renderer
   * disabled (D59), which means software OpenGL, and a bloom pass over a full-size buffer there
   * costs more per frame than everything else in the scene together.
   */
  bloom?: boolean;
}

export class BrainScene {
  readonly renderer: THREE.WebGLRenderer;
  readonly scene = new THREE.Scene();
  readonly camera: THREE.PerspectiveCamera;
  readonly controls: OrbitControls;
  readonly positions: Positions;

  private composer: EffectComposer;
  private bloomPass: UnrealBloomPass;
  private bloomEnabled = true;

  private nodes: THREE.InstancedMesh;
  private halo: THREE.Points;
  private pulses: THREE.Points;
  private edges: THREE.LineSegments;
  private selection: THREE.Mesh;

  private snapshot: GraphSnapshot;
  private indexById = new Map<number, number>();
  private current: Float32Array;
  private from: Float32Array;
  private to: Float32Array;
  private transitionStart = 0;
  private transitioning = false;
  private mode: LayoutMode = "tree";

  /** Per-node multiplier the animation drives; 1 is the node's own size. */
  private scaleFactor: Float32Array;
  private baseRadius: Float32Array;
  private edgePairs: Int32Array;
  private edgeAlpha: Float32Array;
  /** Per edge: 0 hides it, 1 draws it fully. The consolidation animation ramps this. */
  private edgeReveal: Float32Array;

  private pulseCapacity = 512;
  private pulseCursor = 0;
  // `THREE.Clock` is deprecated in this version and says so in the console of anything that
  // uses it; `THREE.Timer` is the replacement and lives in the core.
  private timer = new THREE.Timer();
  private elapsed = 0;
  private frames = 0;
  private fpsWindow = 0;
  fps = 60;

  private dummy = new THREE.Object3D();
  private raycaster = new THREE.Raycaster();
  private pointer = new THREE.Vector2();

  constructor(private options: SceneOptions) {
    this.snapshot = options.snapshot;
    const nodeList = this.snapshot.nodes;
    nodeList.forEach((node, i) => this.indexById.set(node.id, i));

    this.positions = layout(nodeList, this.snapshot.edges);
    this.current = Float32Array.from(this.positions.tree);
    this.from = Float32Array.from(this.positions.tree);
    this.to = Float32Array.from(this.positions.tree);

    this.scene.background = new THREE.Color(BACKGROUND);
    this.scene.fog = new THREE.FogExp2(BACKGROUND, 0.0011);

    this.camera = new THREE.PerspectiveCamera(
      52,
      options.container.clientWidth / Math.max(1, options.container.clientHeight),
      0.5,
      6000,
    );
    const frame = this.positions.extent * 2.1;
    this.camera.position.set(frame * 0.55, frame * 0.42, frame * 0.85);

    this.renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: "high-performance" });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.75));
    this.renderer.setSize(options.container.clientWidth, options.container.clientHeight);
    options.container.appendChild(this.renderer.domElement);

    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = true;
    this.controls.dampingFactor = 0.07;
    this.controls.rotateSpeed = 0.6;
    this.controls.maxDistance = this.positions.extent * 6;

    const zoneHue = new Map<number, number>();
    for (const zone of this.snapshot.zones) zoneHue.set(zone.id, zone.hue);

    // --- nodes -----------------------------------------------------------
    const geometry = new THREE.IcosahedronGeometry(1, 1);
    const material = new THREE.MeshBasicMaterial({ toneMapped: false });
    this.nodes = new THREE.InstancedMesh(geometry, material, nodeList.length);
    this.nodes.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    this.nodes.frustumCulled = false;

    this.baseRadius = new Float32Array(nodeList.length);
    this.scaleFactor = new Float32Array(nodeList.length).fill(1);
    const color = new THREE.Color();
    for (let i = 0; i < nodeList.length; i++) {
      const node = nodeList[i];
      this.baseRadius[i] = nodeRadius(node.salience, node.kind);
      const rgb = nodeColor(node.zone === null ? undefined : zoneHue.get(node.zone), node.tier, node.kind);
      color.setHex(toHex(rgb));
      this.nodes.setColorAt(i, color);
    }
    this.writeInstances();
    this.scene.add(this.nodes);

    // --- halo ------------------------------------------------------------
    const haloGeometry = new THREE.BufferGeometry();
    haloGeometry.setAttribute("position", new THREE.BufferAttribute(this.current, 3));
    const haloSize = new Float32Array(nodeList.length);
    const haloColor = new Float32Array(nodeList.length * 3);
    const haloAlpha = new Float32Array(nodeList.length);
    const working = new Set(this.snapshot.working_set);
    for (let i = 0; i < nodeList.length; i++) {
      const node = nodeList[i];
      const rgb = nodeColor(node.zone === null ? undefined : zoneHue.get(node.zone), node.tier, node.kind);
      haloColor[i * 3] = rgb.r;
      haloColor[i * 3 + 1] = rgb.g;
      haloColor[i * 3 + 2] = rgb.b;
      const isWorking = working.has(node.id);
      // Only the Working Set and pinned nodes get a real halo. Giving every node one is what
      // turned the picture into fog: three thousand overlapping additive sprites add up to a
      // white background no matter how faint each one is.
      haloSize[i] = this.baseRadius[i] * (isWorking ? 7 : node.pinned ? 9 : 2.4);
      haloAlpha[i] = isWorking ? 0.5 : node.pinned ? 0.7 : Math.min(0.1, 0.02 + node.salience * 0.02);
    }
    haloGeometry.setAttribute("size", new THREE.BufferAttribute(haloSize, 1));
    haloGeometry.setAttribute("haloColor", new THREE.BufferAttribute(haloColor, 3));
    haloGeometry.setAttribute("alpha", new THREE.BufferAttribute(haloAlpha, 1));
    this.halo = new THREE.Points(
      haloGeometry,
      new THREE.ShaderMaterial({
        uniforms: { map: { value: haloTexture() } },
        vertexShader: HALO_VERTEX,
        fragmentShader: HALO_FRAGMENT,
        transparent: true,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      }),
    );
    this.halo.frustumCulled = false;
    this.halo.visible = nodeList.length <= HALO_BUDGET;
    this.scene.add(this.halo);

    // --- edges -----------------------------------------------------------
    const drawn = this.snapshot.edges.filter(
      (e) => this.indexById.has(e.src) && this.indexById.has(e.dst),
    );
    this.edgePairs = new Int32Array(drawn.length * 2);
    this.edgeAlpha = new Float32Array(drawn.length);
    this.edgeReveal = new Float32Array(drawn.length).fill(1);
    const edgePositions = new Float32Array(drawn.length * 6);
    const edgeColors = new Float32Array(drawn.length * 6);
    drawn.forEach((edge, i) => {
      const a = this.indexById.get(edge.src)!;
      const b = this.indexById.get(edge.dst)!;
      this.edgePairs[i * 2] = a;
      this.edgePairs[i * 2 + 1] = b;
      const style = EDGE_STYLE[edge.type] ?? EDGE_FALLBACK;
      this.edgeAlpha[i] = style.alpha * (0.45 + 0.55 * Math.min(1, edge.weight));
      for (const end of [0, 1]) {
        const o = i * 6 + end * 3;
        edgeColors[o] = style.color.r;
        edgeColors[o + 1] = style.color.g;
        edgeColors[o + 2] = style.color.b;
      }
    });
    const edgeGeometry = new THREE.BufferGeometry();
    edgeGeometry.setAttribute("position", new THREE.BufferAttribute(edgePositions, 3));
    edgeGeometry.setAttribute("color", new THREE.BufferAttribute(edgeColors, 3));
    this.edges = new THREE.LineSegments(
      edgeGeometry,
      new THREE.LineBasicMaterial({
        vertexColors: true,
        transparent: true,
        opacity: 0.4,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      }),
    );
    this.edges.frustumCulled = false;
    this.drawnEdges = drawn;
    this.writeEdges();
    this.scene.add(this.edges);

    // --- pulses ----------------------------------------------------------
    const pulseGeometry = new THREE.BufferGeometry();
    pulseGeometry.setAttribute(
      "position",
      new THREE.BufferAttribute(new Float32Array(this.pulseCapacity * 3), 3),
    );
    pulseGeometry.setAttribute(
      "born",
      new THREE.BufferAttribute(new Float32Array(this.pulseCapacity).fill(-999), 1),
    );
    pulseGeometry.setAttribute(
      "speed",
      new THREE.BufferAttribute(new Float32Array(this.pulseCapacity).fill(1), 1),
    );
    pulseGeometry.setAttribute(
      "pulseColor",
      new THREE.BufferAttribute(new Float32Array(this.pulseCapacity * 3), 3),
    );
    this.pulses = new THREE.Points(
      pulseGeometry,
      new THREE.ShaderMaterial({
        uniforms: { map: { value: haloTexture() }, now: { value: 0 } },
        vertexShader: PULSE_VERTEX,
        fragmentShader: HALO_FRAGMENT,
        transparent: true,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      }),
    );
    this.pulses.frustumCulled = false;
    this.scene.add(this.pulses);

    // --- selection marker ------------------------------------------------
    this.selection = new THREE.Mesh(
      new THREE.RingGeometry(1.6, 1.9, 40),
      new THREE.MeshBasicMaterial({ color: 0xffffff, side: THREE.DoubleSide, transparent: true }),
    );
    this.selection.visible = false;
    this.scene.add(this.selection);

    // --- post ------------------------------------------------------------
    this.composer = new EffectComposer(this.renderer);
    this.composer.addPass(new RenderPass(this.scene, this.camera));
    // Strength, radius, threshold. The first version used 0.62/0.85/0.22 and produced a white
    // blob: with additive halos underneath, a low threshold means every node contributes to
    // every other node's glow and the hues — which carry the zones — disappear. The threshold
    // now sits above the ordinary node colour, so only the genuinely bright things bloom.
    this.bloomPass = new UnrealBloomPass(
      new THREE.Vector2(options.container.clientWidth, options.container.clientHeight),
      0.34,
      0.72,
      0.62,
    );
    this.composer.addPass(this.bloomPass);
    if (options.bloom === false) this.setBloom(false);

    window.addEventListener("resize", this.onResize);
    // And the container's own size, which in the standalone view is the window's and inside the
    // Agent Inspector is a panel's.
    //
    // **This is why the Inspector's Architektur tab drew nothing.** The scene measured its
    // container once, at construction — and the panel is mounted while it is still animating
    // in, so the measurement was zero. Nothing resized the window afterwards, so nothing ever
    // corrected it: a canvas of 0×0, a black box, and no error anywhere. A `window` listener is
    // the right listener for a page that *is* the view and the wrong one for a view inside a
    // panel.
    if (typeof ResizeObserver !== "undefined") {
      this.containerResize = new ResizeObserver(() => this.onResize());
      this.containerResize.observe(options.container);
    }
  }

  private drawnEdges: GraphEdge[] = [];

  /** The edges actually in the buffer — the view's own legend counts these, not the totals. */
  get visibleEdgeCount(): number {
    return this.drawnEdges.length;
  }

  private containerResize: ResizeObserver | null = null;

  private onResize = (): void => {
    const { container } = this.options;
    const width = container.clientWidth;
    const height = Math.max(1, container.clientHeight);
    // A container with no width yet — mid-animation, or on a tab that has not been shown — is
    // not a size to render at. Skipping keeps the last good one until there is a real one.
    if (width === 0) return;
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    this.renderer.setSize(width, height);
    this.composer.setSize(width, height);
  };

  private writeInstances(): void {
    for (let i = 0; i < this.baseRadius.length; i++) {
      const scale = this.baseRadius[i] * this.scaleFactor[i];
      this.dummy.position.set(this.current[i * 3], this.current[i * 3 + 1], this.current[i * 3 + 2]);
      this.dummy.scale.setScalar(Math.max(0.001, scale));
      this.dummy.updateMatrix();
      this.nodes.setMatrixAt(i, this.dummy.matrix);
    }
    this.nodes.instanceMatrix.needsUpdate = true;
  }

  private writeEdges(): void {
    const positions = this.edges.geometry.getAttribute("position") as THREE.BufferAttribute;
    const array = positions.array as Float32Array;
    for (let i = 0; i < this.edgePairs.length / 2; i++) {
      const a = this.edgePairs[i * 2];
      const b = this.edgePairs[i * 2 + 1];
      const reveal = this.edgeReveal[i];
      const o = i * 6;
      array[o] = this.current[a * 3];
      array[o + 1] = this.current[a * 3 + 1];
      array[o + 2] = this.current[a * 3 + 2];
      // A hidden edge is collapsed onto its own start rather than removed: the buffer keeps a
      // fixed layout, so revealing it again is a write and not a reallocation.
      array[o + 3] = this.current[a * 3] + (this.current[b * 3] - this.current[a * 3]) * reveal;
      array[o + 4] =
        this.current[a * 3 + 1] + (this.current[b * 3 + 1] - this.current[a * 3 + 1]) * reveal;
      array[o + 5] =
        this.current[a * 3 + 2] + (this.current[b * 3 + 2] - this.current[a * 3 + 2]) * reveal;
    }
    positions.needsUpdate = true;
    (this.halo.geometry.getAttribute("position") as THREE.BufferAttribute).needsUpdate = true;
  }

  setLayout(mode: LayoutMode, animate = true): void {
    if (mode === this.mode) return;
    this.mode = mode;
    // Containment is meaningless in the shells: a parent and its child sit on different shells
    // by construction, so every `contains` edge becomes a line across the whole scene and the
    // picture turns into a starburst. The shells answer a different question and only the
    // association edges belong in them.
    this.setRevealForType("contains", mode === "tree" ? 1 : 0);
    const target = mode === "tree" ? this.positions.tree : this.positions.shells;
    if (!animate) {
      this.current.set(target);
      this.writeInstances();
      this.writeEdges();
      return;
    }
    this.from.set(this.current);
    this.to.set(target);
    this.transitionStart = this.elapsed;
    this.transitioning = true;
  }

  get layoutMode(): LayoutMode {
    return this.mode;
  }

  setBloom(enabled: boolean): void {
    this.bloomEnabled = enabled;
    this.bloomPass.enabled = enabled;
  }

  get bloom(): boolean {
    return this.bloomEnabled;
  }

  /** Scale one node, for the scoring phase of the consolidation animation. */
  setScale(nodeId: number, factor: number): void {
    const i = this.indexById.get(nodeId);
    if (i === undefined) return;
    this.scaleFactor[i] = factor;
  }

  resetScales(): void {
    this.scaleFactor.fill(1);
  }

  /**
   * Flatten every node towards one size, so that scoring can be watched happening.
   *
   * `1` draws every node identically — the index before it has an opinion — and `0` draws each
   * at the size its salience earned. The scoring phase animates from one to the other, which is
   * the only way to *see* a ranking appear rather than be told about it.
   */
  setUniformity(u: number): void {
    const uniform = 1.1;
    const t = Math.max(0, Math.min(1, u));
    for (let i = 0; i < this.baseRadius.length; i++) {
      this.scaleFactor[i] = 1 + (uniform / this.baseRadius[i] - 1) * t;
    }
  }

  /** Hide every association edge, so the animation can draw them in one at a time. */
  setRevealForType(type: string, reveal: number): void {
    for (let i = 0; i < this.drawnEdges.length; i++) {
      if (this.drawnEdges[i].type === type) this.edgeReveal[i] = reveal;
    }
  }

  revealFraction(type: string, fraction: number): void {
    const matching: number[] = [];
    for (let i = 0; i < this.drawnEdges.length; i++) {
      if (this.drawnEdges[i].type === type) matching.push(i);
    }
    const upTo = Math.floor(matching.length * Math.max(0, Math.min(1, fraction)));
    matching.forEach((edge, order) => {
      this.edgeReveal[edge] = order < upTo ? 1 : 0;
    });
  }

  /** A ring at a node. Colour is the node's own, so a burst reads as one place lighting up. */
  pulse(nodeId: number, speed = 0.9): void {
    const i = this.indexById.get(nodeId);
    if (i === undefined) return;
    const slot = this.pulseCursor % this.pulseCapacity;
    this.pulseCursor++;
    const geometry = this.pulses.geometry;
    const position = geometry.getAttribute("position") as THREE.BufferAttribute;
    const born = geometry.getAttribute("born") as THREE.BufferAttribute;
    const speedAttr = geometry.getAttribute("speed") as THREE.BufferAttribute;
    const color = geometry.getAttribute("pulseColor") as THREE.BufferAttribute;
    position.setXYZ(slot, this.current[i * 3], this.current[i * 3 + 1], this.current[i * 3 + 2]);
    born.setX(slot, this.elapsed);
    speedAttr.setX(slot, speed);
    const instanceColor = this.nodes.instanceColor;
    if (instanceColor) {
      color.setXYZ(
        slot,
        instanceColor.getX(i),
        instanceColor.getY(i),
        instanceColor.getZ(i),
      );
    } else {
      color.setXYZ(slot, 1, 1, 1);
    }
    position.needsUpdate = true;
    born.needsUpdate = true;
    speedAttr.needsUpdate = true;
    color.needsUpdate = true;
  }

  /** The node under the pointer, or null. */
  pick(clientX: number, clientY: number): GraphNode | null {
    const rect = this.renderer.domElement.getBoundingClientRect();
    this.pointer.x = ((clientX - rect.left) / rect.width) * 2 - 1;
    this.pointer.y = -((clientY - rect.top) / rect.height) * 2 + 1;
    this.raycaster.setFromCamera(this.pointer, this.camera);
    const hits = this.raycaster.intersectObject(this.nodes, false);
    if (hits.length === 0) return null;
    const instanceId = hits[0].instanceId;
    if (instanceId === undefined) return null;
    return this.snapshot.nodes[instanceId] ?? null;
  }

  select(nodeId: number | null): void {
    if (nodeId === null) {
      this.selection.visible = false;
      return;
    }
    const i = this.indexById.get(nodeId);
    if (i === undefined) {
      this.selection.visible = false;
      return;
    }
    const scale = this.baseRadius[i] * this.scaleFactor[i] * 2.4;
    this.selection.position.set(
      this.current[i * 3],
      this.current[i * 3 + 1],
      this.current[i * 3 + 2],
    );
    this.selection.scale.setScalar(Math.max(1.5, scale));
    this.selection.visible = true;
  }

  /** Move the camera to look at a node without losing the current distance. */
  focus(nodeId: number): void {
    const i = this.indexById.get(nodeId);
    if (i === undefined) return;
    const target = new THREE.Vector3(
      this.current[i * 3],
      this.current[i * 3 + 1],
      this.current[i * 3 + 2],
    );
    const offset = this.camera.position.clone().sub(this.controls.target);
    this.controls.target.copy(target);
    this.camera.position.copy(target).add(offset);
  }

  resetCamera(): void {
    const frame = this.positions.extent * 2.1;
    this.controls.target.set(0, 0, 0);
    this.camera.position.set(frame * 0.55, frame * 0.42, frame * 0.85);
  }

  /** One frame. Returns the seconds since the previous one. */
  render(onFrame?: (dt: number) => void): number {
    this.timer.update();
    const dt = this.timer.getDelta();
    this.elapsed += dt;

    if (this.transitioning) {
      const t = Math.min(1, ((this.elapsed - this.transitionStart) * 1000) / TRANSITION_MS);
      const eased = easeInOut(t);
      for (let i = 0; i < this.current.length; i++) {
        this.current[i] = this.from[i] + (this.to[i] - this.from[i]) * eased;
      }
      this.writeInstances();
      this.writeEdges();
      if (t >= 1) this.transitioning = false;
    }

    onFrame?.(dt);
    this.selection.lookAt(this.camera.position);
    (this.pulses.material as THREE.ShaderMaterial).uniforms.now.value = this.elapsed;
    this.controls.update();
    // The composer runs every pass it holds, whether or not they do anything. With the glow off
    // there is nothing for it to do that the renderer cannot do directly, and on a software
    // renderer the difference is the difference between a usable panel and a frozen one.
    if (this.bloomEnabled) this.composer.render();
    else this.renderer.render(this.scene, this.camera);

    this.frames++;
    this.fpsWindow += dt;
    if (this.fpsWindow >= 0.5) {
      this.fps = this.frames / this.fpsWindow;
      this.frames = 0;
      this.fpsWindow = 0;
    }
    return dt;
  }

  /** Push scale changes made through `setScale` into the instance matrices. */
  commitScales(): void {
    this.writeInstances();
  }

  commitEdges(): void {
    this.writeEdges();
  }

  get time(): number {
    return this.elapsed;
  }

  dispose(): void {
    window.removeEventListener("resize", this.onResize);
    this.containerResize?.disconnect();
    this.containerResize = null;
    this.renderer.dispose();
    this.controls.dispose();
  }
}
