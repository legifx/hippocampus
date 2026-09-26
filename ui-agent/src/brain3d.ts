/**
 * The brain, drawn as a brain: the live half of the Agent View.
 *
 * `scene.ts` answers "what does the index believe" with a tree and concentric shells. This file
 * answers "what is happening in it right now", and draws it in the shape the project is named
 * after, because that shape carries the model:
 *
 * - **The hippocampus** sits at the centre: a pair of curved arcs. It is where the working set
 *   lives and where every agent's request enters — each call is drawn as a spark that leaves it.
 * - **The cortex** is two hemispheres. Every zone (a project, an area) owns a region of it, at a
 *   fixed place: the regions are laid out on a golden spiral in zone-id order, so the same
 *   project is in the same place every time the view opens.
 * - **Depth is consolidation.** A node sits at a depth set by its tier — Working close to the
 *   hippocampus, Short-term further out, Long-term near the surface, Cold at the surface itself.
 *   Forgetting moves a node outward; nothing else in the picture moves on its own.
 * - **Heat** is a glow on a region: how much it is being asked about (cool) and written to (warm)
 *   right now, with the half-life the daemon decays it with.
 *
 * Cost is the design constraint, not an afterthought. On the target machine any continuously
 * running animation recomposites the whole window every frame (D59), so this scene **renders on
 * demand**: while a spark is travelling, a glow is fading, or the camera is being moved — and not
 * otherwise. Idle, visible and still, it draws nothing at all. Hidden, it does not even schedule a
 * frame. Everything is one draw call per layer: points with a small shader, no post-processing.
 */

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { GraphNode, GraphSnapshot, Tier } from "./snapshot";

export interface HeatInput {
  zone: string;
  heat: number;
  reads: number;
  writes: number;
}

export interface PulseInput {
  zone: string | null;
  kind: string;
  paths: string[];
}

export interface Brain3DOptions {
  container: HTMLElement;
  snapshot: GraphSnapshot;
  /** Called with the zone under the pointer, for a caption. Optional. */
  onHover?: (zone: string | null) => void;
}

const TIER_DEPTH: Record<Tier, number> = { working: 0.34, stm: 0.74, ltm: 0.87, cold: 0.97 };
const TIER_LIGHT: Record<Tier, number> = { working: 0.78, stm: 0.62, ltm: 0.52, cold: 0.36 };
/** The brain's half-extents: wider than tall, longer front to back. */
const RX = 62;
const RY = 50;
const RZ = 72;
const PULSE_MS = 1100;
const FLASH_MS = 900;
const FADE_MS = 700;
const FRAME_MS = 33;
const MAX_PULSES = 64;
const GOLDEN = Math.PI * (3 - Math.sqrt(5));

/** A small, fast, deterministic hash: the same node lands in the same place on every machine. */
function hash01(n: number, salt: number): number {
  let x = (n ^ (salt * 0x9e3779b1)) >>> 0;
  x = Math.imul(x ^ (x >>> 16), 0x45d9f3b) >>> 0;
  x = Math.imul(x ^ (x >>> 16), 0x45d9f3b) >>> 0;
  x = (x ^ (x >>> 16)) >>> 0;
  return x / 4294967296;
}

function hsl(h: number, s: number, l: number): THREE.Color {
  return new THREE.Color().setHSL(((h % 360) + 360) % 360 / 360, s, l);
}

/** A point on the brain's surface in direction `d`, with shallow folds so it reads as cortex. */
function surface(d: THREE.Vector3, out = new THREE.Vector3()): THREE.Vector3 {
  const theta = Math.atan2(d.z, d.x);
  const phi = Math.acos(Math.max(-1, Math.min(1, d.y)));
  const fold = 1 + 0.035 * Math.sin(9 * theta + 2 * phi) * Math.cos(6 * phi) + 0.02 * Math.sin(17 * phi);
  // The longitudinal fissure: a groove along the midline between the hemispheres.
  const fissure = 1 - 0.07 * Math.exp(-Math.pow(d.x / 0.08, 2)) * Math.max(0, d.y);
  const r = fold * fissure;
  return out.set(d.x * RX * r, d.y * RY * r, d.z * RZ * r);
}

const VERTEX = /* glsl */ `
  attribute float size;
  attribute vec3 color;
  attribute float alpha;
  uniform float scale;
  varying vec3 vColor;
  varying float vAlpha;
  void main() {
    vColor = color;
    vAlpha = alpha;
    vec4 mv = modelViewMatrix * vec4(position, 1.0);
    gl_PointSize = size * scale / -mv.z;
    gl_Position = projectionMatrix * mv;
  }
`;

const FRAGMENT = /* glsl */ `
  varying vec3 vColor;
  varying float vAlpha;
  void main() {
    vec2 c = gl_PointCoord - 0.5;
    float d = dot(c, c);
    if (d > 0.25) discard;
    float a = vAlpha * smoothstep(0.25, 0.0, d);
    gl_FragColor = vec4(vColor * a, a);
  }
`;

interface Layer {
  points: THREE.Points;
  position: THREE.BufferAttribute;
  color: THREE.BufferAttribute;
  size: THREE.BufferAttribute;
  alpha: THREE.BufferAttribute;
}

function layer(count: number, material: THREE.ShaderMaterial): Layer {
  const geometry = new THREE.BufferGeometry();
  const position = new THREE.BufferAttribute(new Float32Array(count * 3), 3);
  const color = new THREE.BufferAttribute(new Float32Array(count * 3), 3);
  const size = new THREE.BufferAttribute(new Float32Array(count), 1);
  const alpha = new THREE.BufferAttribute(new Float32Array(count), 1);
  geometry.setAttribute("position", position);
  geometry.setAttribute("color", color);
  geometry.setAttribute("size", size);
  geometry.setAttribute("alpha", alpha);
  const points = new THREE.Points(geometry, material);
  points.frustumCulled = false;
  return { points, position, color, size, alpha };
}

interface Region {
  name: string;
  index: number;
  hue: number;
  center: THREE.Vector3;
  direction: THREE.Vector3;
  heat: number;
  from: number;
  to: number;
  fadeStart: number;
  flash: number;
  flashStart: number;
  writeShare: number;
}

interface Spark {
  start: number;
  from: THREE.Vector3;
  via: THREE.Vector3;
  to: THREE.Vector3;
  color: THREE.Color;
  region: Region | null;
}

export class Brain3D {
  private readonly renderer: THREE.WebGLRenderer;
  private readonly scene = new THREE.Scene();
  private readonly camera: THREE.PerspectiveCamera;
  private readonly controls: OrbitControls;
  private readonly material: THREE.ShaderMaterial;
  private readonly haloMaterial: THREE.ShaderMaterial;
  private readonly regions: Region[] = [];
  private readonly regionByName = new Map<string, Region>();
  private readonly nodeByPath = new Map<string, THREE.Vector3>();
  /** Each zone's share of the drawn nodes: a big project gets a bigger patch of cortex. */
  private readonly zoneShare = new Map<string, number>();
  /** The largest zones, labelled always, so the picture can be read without hovering. */
  private readonly landmarks: Region[] = [];
  private readonly halos: Layer;
  private readonly sparks: Layer;
  private readonly live: Spark[] = [];
  private readonly core: THREE.Vector3[] = [];
  private readonly labels: HTMLDivElement;
  private readonly resize: ResizeObserver;
  private visible = true;
  private ambient = false;
  private interacting = false;
  private settleUntil = 0;
  private frame: number | null = null;
  private lastFrame = 0;
  private framesDrawn = 0;
  private disposed = false;

  constructor(private readonly options: Brain3DOptions) {
    const { container, snapshot } = options;
    this.renderer = new THREE.WebGLRenderer({ antialias: false, alpha: true, powerPreference: "low-power" });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.5));
    this.renderer.setClearColor(0x000000, 0);
    container.appendChild(this.renderer.domElement);
    this.renderer.domElement.style.display = "block";

    this.camera = new THREE.PerspectiveCamera(42, 1, 1, 2000);
    this.camera.position.set(108, 52, 122);
    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = true;
    this.controls.dampingFactor = 0.08;
    this.controls.minDistance = 80;
    this.controls.maxDistance = 420;
    this.controls.enablePan = false;
    this.controls.autoRotateSpeed = 0.35;
    this.controls.addEventListener("start", () => {
      this.interacting = true;
      this.wake();
    });
    this.controls.addEventListener("end", () => {
      this.interacting = false;
      this.settleUntil = performance.now() + 1200;
      this.wake();
    });
    this.controls.addEventListener("change", () => this.wake());

    const uniforms = () => ({ scale: { value: 400 } });
    this.material = new THREE.ShaderMaterial({
      vertexShader: VERTEX,
      fragmentShader: FRAGMENT,
      uniforms: uniforms(),
      transparent: true,
      depthWrite: false,
      blending: THREE.AdditiveBlending,
    });
    this.haloMaterial = this.material.clone();

    this.buildShell();
    this.buildCore();
    this.buildRegions(snapshot);
    this.buildNodes(snapshot.nodes);
    this.halos = layer(Math.max(1, this.regions.length), this.haloMaterial);
    this.scene.add(this.halos.points);
    this.sparks = layer(MAX_PULSES, this.material);
    this.scene.add(this.sparks.points);
    this.writeHalos(performance.now());

    this.labels = document.createElement("div");
    this.labels.className = "hc-brain-labels";
    container.appendChild(this.labels);

    this.resize = new ResizeObserver(() => this.fit());
    this.resize.observe(container);
    this.fit();
  }

  // --- construction ---------------------------------------------------------------------------

  private buildShell(): void {
    // The silhouette: a few thousand faint points on the folded surface. Static, uploaded once.
    const count = 5200;
    const shell = layer(count, this.material);
    const d = new THREE.Vector3();
    const p = new THREE.Vector3();
    for (let i = 0; i < count; i++) {
      const y = 1 - (2 * (i + 0.5)) / count;
      const r = Math.sqrt(1 - y * y);
      const a = i * GOLDEN;
      d.set(Math.cos(a) * r, y, Math.sin(a) * r);
      // Flatten the underside a little: a brain rests on its base, it is not a ball.
      if (d.y < -0.55) d.y = -0.55 - (d.y + 0.55) * 0.35;
      surface(d.normalize(), p);
      shell.position.setXYZ(i, p.x, p.y, p.z);
      const c = hsl(210, 0.4, 0.62);
      shell.color.setXYZ(i, c.r, c.g, c.b);
      shell.size.setX(i, 2.6);
      // Brighter towards the rim as seen from the side: the outline is what makes it a brain.
      shell.alpha.setX(i, 0.2 + 0.16 * hash01(i, 7));
    }
    this.scene.add(shell.points);
  }

  private buildCore(): void {
    // Two mirrored arcs low in the middle: the hippocampus, where requests enter.
    const count = 360;
    const core = layer(count, this.material);
    for (let i = 0; i < count; i++) {
      const side = i % 2 === 0 ? 1 : -1;
      const t = Math.floor(i / 2) / (count / 2 - 1);
      const angle = -0.6 + t * 2.4;
      const x = side * (10 + 4 * Math.sin(t * Math.PI));
      const y = -14 + 9 * Math.sin(angle);
      const z = -8 + 22 * Math.cos(angle) - 6;
      const jitter = 1.6;
      const px = x + (hash01(i, 1) - 0.5) * jitter;
      const py = y + (hash01(i, 2) - 0.5) * jitter;
      const pz = z + (hash01(i, 3) - 0.5) * jitter;
      core.position.setXYZ(i, px, py, pz);
      if (i % 12 === 0) this.core.push(new THREE.Vector3(px, py, pz));
      const c = hsl(172, 0.7, 0.62);
      core.color.setXYZ(i, c.r, c.g, c.b);
      core.size.setX(i, 3.2);
      core.alpha.setX(i, 0.55);
    }
    this.scene.add(core.points);
  }

  private buildRegions(snapshot: GraphSnapshot): void {
    const zones = [...(snapshot.zones ?? [])].sort((a, b) => a.id - b.id);
    const n = Math.max(zones.length, 1);
    zones.forEach((zone, index) => {
      // A golden spiral over the upper three quarters of the sphere: the base is brain stem.
      const y = 0.92 - (1.55 * (index + 0.5)) / n;
      const r = Math.sqrt(Math.max(0, 1 - y * y));
      const a = index * GOLDEN + 0.4;
      const direction = new THREE.Vector3(Math.cos(a) * r, y, Math.sin(a) * r).normalize();
      // Keep regions off the midline so each sits in a hemisphere.
      if (Math.abs(direction.x) < 0.18) direction.x = direction.x < 0 ? -0.18 : 0.18;
      direction.normalize();
      const center = surface(direction).multiplyScalar(0.9);
      const region: Region = {
        name: zone.name,
        index,
        hue: zone.hue,
        center,
        direction,
        heat: 0,
        from: 0,
        to: 0,
        fadeStart: 0,
        flash: 0,
        flashStart: 0,
        writeShare: 0,
      };
      this.regions.push(region);
      this.regionByName.set(zone.name, region);
    });
  }

  private buildNodes(nodes: GraphNode[]): void {
    const count = nodes.length;
    if (count === 0) return;
    const layerNodes = layer(count, this.material);
    const byId = new Map<number, Region>();
    const zones = [...this.regions];
    const regionOf = (zoneId: number | null): Region | undefined => {
      if (zoneId === null) return undefined;
      if (byId.has(zoneId)) return byId.get(zoneId);
      return undefined;
    };
    // Region lookup by the snapshot's zone id, and each zone's share of what is drawn.
    const counts = new Map<number, number>();
    for (const node of nodes) if (node.zone !== null) counts.set(node.zone, (counts.get(node.zone) ?? 0) + 1);
    for (const z of this.options.snapshot.zones ?? []) {
      const region = this.regionByName.get(z.name);
      if (region) {
        byId.set(z.id, region);
        this.zoneShare.set(z.name, (counts.get(z.id) ?? 0) / Math.max(1, count));
      }
    }
    this.landmarks.push(
      ...[...this.regions]
        .filter((r) => (this.zoneShare.get(r.name) ?? 0) > 0.015)
        .sort((a, b) => (this.zoneShare.get(b.name) ?? 0) - (this.zoneShare.get(a.name) ?? 0))
        .slice(0, 10),
    );
    const d = new THREE.Vector3();
    const p = new THREE.Vector3();
    nodes.forEach((node, i) => {
      const region = regionOf(node.zone) ?? zones[0];
      const base = region ? region.direction : new THREE.Vector3(0, 1, 0);
      // Scatter inside a cone around the region's direction, deterministically.
      const u = hash01(node.id, 11);
      const v = hash01(node.id, 13);
      const share = region ? (this.zoneShare.get(region.name) ?? 0.02) : 0.02;
      const spread = 0.16 + 0.55 * Math.sqrt(share);
      const tangent = new THREE.Vector3(-base.z, 0, base.x).normalize();
      if (tangent.lengthSq() < 1e-6) tangent.set(1, 0, 0);
      const bitangent = new THREE.Vector3().crossVectors(base, tangent).normalize();
      const angle = v * Math.PI * 2;
      const radius = Math.sqrt(u) * spread;
      d.copy(base)
        .addScaledVector(tangent, Math.cos(angle) * radius)
        .addScaledVector(bitangent, Math.sin(angle) * radius)
        .normalize();
      const tier = (node.tier in TIER_DEPTH ? node.tier : "stm") as Tier;
      const depth = TIER_DEPTH[tier] * (0.94 + 0.08 * hash01(node.id, 17));
      surface(d, p).multiplyScalar(depth);
      layerNodes.position.setXYZ(i, p.x, p.y, p.z);
      if (node.path) this.nodeByPath.set(node.path, p.clone());

      let c: THREE.Color;
      if (node.kind === "concept") c = hsl(45, 0.9, 0.62);
      else if (node.kind === "memory") c = hsl(140, 0.7, 0.6);
      else c = hsl(region ? region.hue : 210, 0.55, TIER_LIGHT[tier]);
      layerNodes.color.setXYZ(i, c.r, c.g, c.b);
      const salience = Math.max(0, node.salience || 0);
      const big = node.kind === "concept" || node.kind === "memory" ? 2.2 : 0;
      layerNodes.size.setX(i, Math.min(9, 2.4 + Math.sqrt(salience) * 1.1 + big));
      layerNodes.alpha.setX(i, node.kind === "dir" ? 0.35 : 0.75);
    });
    this.scene.add(layerNodes.points);
  }

  // --- the live half --------------------------------------------------------------------------

  /** New heat numbers from the daemon. The glow fades to them rather than jumping. */
  setHeat(zones: HeatInput[]): void {
    const now = performance.now();
    const max = Math.max(1, ...zones.map((z) => z.heat));
    const seen = new Set<string>();
    for (const z of zones) {
      const region = this.regionByName.get(z.zone);
      if (!region) continue;
      seen.add(z.zone);
      const target = Math.min(1, Math.sqrt(z.heat / max));
      region.writeShare = z.reads + z.writes > 0 ? (2 * z.writes) / (z.reads + 2 * z.writes) : 0;
      if (Math.abs(target - region.to) > 0.01) {
        region.from = this.currentHeat(region, now);
        region.to = target;
        region.fadeStart = now;
      }
    }
    for (const region of this.regions) {
      if (!seen.has(region.name) && region.to !== 0) {
        region.from = this.currentHeat(region, now);
        region.to = 0;
        region.fadeStart = now;
      }
    }
    this.wake();
  }

  /** One call, drawn as a spark from the hippocampus to where it went. */
  pulse(event: PulseInput): void {
    if (!this.visible) return; // a hidden view does not queue a show for later
    const region = event.zone ? this.regionByName.get(event.zone) ?? null : null;
    let target: THREE.Vector3 | null = null;
    for (const path of event.paths) {
      const at = this.nodeByPath.get(path);
      if (at) {
        target = at;
        break;
      }
    }
    if (!target && region) target = region.center;
    if (!target) return;
    const from = this.core[Math.floor(Math.random() * this.core.length)] ?? new THREE.Vector3(0, -14, 0);
    const via = from.clone().add(target).multiplyScalar(0.5);
    via.y += 26;
    const color =
      event.kind === "write" ? hsl(32, 0.95, 0.62) : event.kind === "refused" ? hsl(0, 0.85, 0.6) : hsl(190, 0.9, 0.66);
    if (this.live.length >= MAX_PULSES) this.live.shift();
    this.live.push({ start: performance.now(), from: from.clone(), via, to: target.clone(), color, region });
    this.wake();
  }

  /** Whether anyone can see this. False stops the loop and drops sparks in flight. */
  setVisible(visible: boolean): void {
    this.visible = visible;
    if (!visible) {
      this.live.length = 0;
      if (this.frame !== null) cancelAnimationFrame(this.frame);
      this.frame = null;
    } else {
      this.wake();
    }
  }

  /** A slow rotation, for when somebody wants to watch. Off by default: it is the one thing that
   *  draws every frame for as long as it is on. */
  setAmbient(on: boolean): void {
    this.ambient = on;
    this.controls.autoRotate = on;
    this.wake();
  }

  /** Frames drawn since construction, for a caption that proves the loop stops. */
  get frames(): number {
    return this.framesDrawn;
  }

  dispose(): void {
    this.disposed = true;
    if (this.frame !== null) cancelAnimationFrame(this.frame);
    this.resize.disconnect();
    this.controls.dispose();
    this.scene.traverse((o) => {
      const p = o as THREE.Points;
      p.geometry?.dispose?.();
    });
    this.material.dispose();
    this.haloMaterial.dispose();
    this.renderer.dispose();
    this.renderer.domElement.remove();
    this.labels.remove();
  }

  // --- the loop -------------------------------------------------------------------------------

  private currentHeat(region: Region, now: number): number {
    const t = Math.min(1, (now - region.fadeStart) / FADE_MS);
    return region.from + (region.to - region.from) * (t * (2 - t));
  }

  private busy(now: number): boolean {
    if (this.live.length > 0 || this.interacting || this.ambient || now < this.settleUntil) return true;
    return this.regions.some((r) => now - r.fadeStart < FADE_MS || now - r.flashStart < FLASH_MS);
  }

  private wake(): void {
    if (this.disposed || !this.visible || this.frame !== null) return;
    this.frame = requestAnimationFrame((t) => this.tick(t));
  }

  private tick(now: number): void {
    this.frame = null;
    if (this.disposed || !this.visible) return;
    if (now - this.lastFrame >= FRAME_MS) {
      this.lastFrame = now;
      this.draw(now);
    }
    if (this.busy(now)) this.frame = requestAnimationFrame((t) => this.tick(t));
  }

  private draw(now: number): void {
    this.controls.update();
    this.updateSparks(now);
    this.writeHalos(now);
    this.renderer.render(this.scene, this.camera);
    this.writeLabels();
    this.framesDrawn += 1;
  }

  private updateSparks(now: number): void {
    const s = this.sparks;
    const p = new THREE.Vector3();
    let n = 0;
    for (let i = this.live.length - 1; i >= 0; i--) {
      const spark = this.live[i];
      const t = (now - spark.start) / PULSE_MS;
      if (t >= 1) {
        if (spark.region) {
          spark.region.flash = 1;
          spark.region.flashStart = now;
        }
        this.live.splice(i, 1);
        continue;
      }
      // A quadratic Bézier: from the hippocampus, up and over, into the region.
      const a = 1 - t;
      p.set(0, 0, 0)
        .addScaledVector(spark.from, a * a)
        .addScaledVector(spark.via, 2 * a * t)
        .addScaledVector(spark.to, t * t);
      s.position.setXYZ(n, p.x, p.y, p.z);
      s.color.setXYZ(n, spark.color.r, spark.color.g, spark.color.b);
      s.size.setX(n, 7 + 5 * Math.sin(t * Math.PI));
      s.alpha.setX(n, 0.95);
      n += 1;
    }
    for (let i = n; i < MAX_PULSES; i++) s.alpha.setX(i, 0);
    s.position.needsUpdate = true;
    s.color.needsUpdate = true;
    s.size.needsUpdate = true;
    s.alpha.needsUpdate = true;
  }

  private writeHalos(now: number): void {
    const h = this.halos;
    this.regions.forEach((region, i) => {
      region.heat = this.currentHeat(region, now);
      const flash = now - region.flashStart < FLASH_MS ? 1 - (now - region.flashStart) / FLASH_MS : 0;
      const intensity = Math.min(1, region.heat + 0.5 * flash);
      h.position.setXYZ(i, region.center.x, region.center.y, region.center.z);
      // Cool for reading, warm for writing, blended by how much of the heat is writes.
      const cool = hsl(region.hue, 0.6, 0.6);
      const warm = hsl(28, 0.95, 0.6);
      const c = cool.lerp(warm, region.writeShare * 0.8);
      h.color.setXYZ(i, c.r, c.g, c.b);
      h.size.setX(i, 26 + 70 * intensity);
      h.alpha.setX(i, intensity < 0.01 ? 0 : 0.12 + 0.55 * intensity);
    });
    h.position.needsUpdate = true;
    h.color.needsUpdate = true;
    h.size.needsUpdate = true;
    h.alpha.needsUpdate = true;
  }

  private writeLabels(): void {
    // The hottest regions by name, bright; the largest projects always, quiet — a map needs
    // landmarks even when nothing is happening.
    const hot = this.regions.filter((r) => r.heat > 0.05).sort((a, b) => b.heat - a.heat).slice(0, 6);
    const shown = new Set(hot);
    const width = this.renderer.domElement.clientWidth;
    const height = this.renderer.domElement.clientHeight;
    const v = new THREE.Vector3();
    const html: string[] = [];
    const place = (r: Region, cls: string) => {
      v.copy(r.center).project(this.camera);
      if (v.z > 1) return;
      const x = (v.x * 0.5 + 0.5) * width;
      const y = (-v.y * 0.5 + 0.5) * height;
      const name = r.name.replace(/[&<>"]/g, "");
      html.push(`<span class="hc-brain-label ${cls}" style="transform:translate(${x.toFixed(0)}px,${y.toFixed(0)}px)">${name}</span>`);
    };
    for (const r of this.landmarks) if (!shown.has(r)) place(r, "hc-brain-label-quiet");
    for (const r of hot) place(r, "hc-brain-label-hot");
    const next = html.join("");
    if (this.labels.innerHTML !== next) this.labels.innerHTML = next;
  }

  private fit(): void {
    const el = this.options.container;
    const w = Math.max(1, el.clientWidth);
    const h = Math.max(1, el.clientHeight);
    this.renderer.setSize(w, h, false);
    this.renderer.domElement.style.width = `${w}px`;
    this.renderer.domElement.style.height = `${h}px`;
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    const scale = (h * this.renderer.getPixelRatio()) / 2;
    this.material.uniforms.scale.value = scale;
    this.haloMaterial.uniforms.scale.value = scale;
    // One frame for the new size; nothing keeps drawing afterwards.
    if (this.visible) this.draw(performance.now());
  }
}
