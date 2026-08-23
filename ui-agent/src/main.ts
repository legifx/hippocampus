/**
 * Wiring. Load a snapshot, build the scene, hand the two of them to the interface.
 *
 * The order matters in one place: nothing is drawn before a snapshot has been validated, so a
 * daemon that answers with something unexpected produces one legible error rather than a black
 * screen with a stack trace behind it.
 */

import "./style.css";
import { BrainScene } from "./scene";
import { Hud } from "./hud";
import { ConsolidationPlayback } from "./consolidation";
import { I18n } from "./i18n";
import { load } from "./snapshot";

const container = document.getElementById("app")!;

function fail(error: unknown): void {
  const i18n = new I18n();
  const box = document.createElement("div");
  box.className = "fatal";
  const title = document.createElement("h1");
  title.textContent = i18n.t("error.title");
  const hint = document.createElement("p");
  hint.textContent = i18n.t("error.hint");
  const detail = document.createElement("pre");
  detail.textContent = error instanceof Error ? error.message : String(error);
  box.append(title, hint, detail);
  container.appendChild(box);
}

async function main(): Promise<void> {
  const { snapshot, origin } = await load();
  const i18n = new I18n("de");

  const scene = new BrainScene({ container, snapshot });
  let playback: ConsolidationPlayback;

  const hud = new Hud(
    container,
    snapshot,
    origin,
    i18n,
    {
      onLayout: (mode) => {
        scene.setLayout(mode);
        hud.setLayoutButton(mode);
      },
      onConsolidate: () => {
        if (playback.isRunning) playback.stop();
        else playback.start();
      },
      onReset: () => {
        playback.stop();
        scene.setLayout("tree");
        hud.setLayoutButton("tree");
        scene.resetCamera();
        scene.select(null);
        hud.setSelected(null);
      },
      onBloom: (enabled) => scene.setBloom(enabled),
      onLanguage: () => {
        i18n.toggle();
        hud.retranslate();
        hud.setLayoutButton(scene.layoutMode);
      },
      onFocus: (id) => scene.focus(id),
    },
    scene.visibleEdgeCount,
  );

  playback = new ConsolidationPlayback(scene, snapshot, i18n, (caption) =>
    hud.setCaption(caption),
  );

  // A click that was part of a drag is not a selection. Three pixels of slop, because a
  // trackpad never gives you exactly zero.
  let downAt: { x: number; y: number } | null = null;
  scene.renderer.domElement.addEventListener("pointerdown", (event) => {
    downAt = { x: event.clientX, y: event.clientY };
  });
  scene.renderer.domElement.addEventListener("pointerup", (event) => {
    if (!downAt) return;
    const moved = Math.hypot(event.clientX - downAt.x, event.clientY - downAt.y);
    downAt = null;
    if (moved > 3) return;
    const node = scene.pick(event.clientX, event.clientY);
    scene.select(node?.id ?? null);
    hud.setSelected(node);
  });

  window.addEventListener("keydown", (event) => {
    if (event.key === "l") {
      const next = scene.layoutMode === "tree" ? "shells" : "tree";
      scene.setLayout(next);
      hud.setLayoutButton(next);
    }
    if (event.key === "c") playback.isRunning ? playback.stop() : playback.start();
    if (event.key === "Escape") {
      playback.stop();
      scene.select(null);
      hud.setSelected(null);
    }
  });

  let sinceHud = 0;
  let shownLayout = scene.layoutMode;
  const frame = (): void => {
    requestAnimationFrame(frame);
    const dt = scene.render((delta) => playback.update(delta));
    // The HUD is text; refreshing it every frame is work nobody can see.
    sinceHud += dt;
    if (sinceHud > 0.5) {
      sinceHud = 0;
      hud.setFps(scene.fps);
      // The consolidation playback changes the layout on its own, so the button follows the
      // scene rather than the other way round.
      if (scene.layoutMode !== shownLayout) {
        shownLayout = scene.layoutMode;
        hud.setLayoutButton(shownLayout);
      }
    }
  };
  frame();
}

main().catch(fail);
