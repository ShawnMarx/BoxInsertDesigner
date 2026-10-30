// Cut view: two section planes through whatever the viewer shows, on the Insert and Tray tabs.
// "Horizontal" sweeps a vertical plane along the longer side of what's shown, from the end
// nearest the default camera, keeping the far side so the cut faces the viewer; "Vertical"
// lowers a horizontal plane from the top. Each has a swap toggle for which side is kept.
//
// Solids are cut solid: each closed mesh (trays, printed boxes, contents) gets a cap where a
// plane cuts it, in its own colour, by the stencil method — the mesh's back faces count up and
// its front faces count down in the stencil buffer, so it is non-zero exactly inside the solid's
// section, and a plane drawn there (then reset to zero) fills it.

import * as THREE from 'three';

export function initCut(ctx) {
  // ctx: { renderer, scene, button, groups: [Group...] (what to cut), viewer }
  const panel = document.createElement('div');
  panel.className = 'cut-panel';
  panel.hidden = true;
  panel.innerHTML = `
    <label>Horizontal <input type="range" name="h" min="0" max="1000" value="0"> <button class="toggle" data-swap="h" aria-pressed="false" title="Keep the other side">⇄</button></label>
    <label>Vertical <input type="range" name="v" min="0" max="1000" value="1000"> <button class="toggle" data-swap="v" aria-pressed="false" title="Keep the other side">⇅</button></label>`;
  ctx.viewer.append(panel);
  // t: where the plane is, 0–1 along its range; high: the side kept is above it.
  const cut = { h: { t: 0, swap: false, high: true }, v: { t: 1, swap: false, high: false } };
  const keepsHigh = (c) => c.high !== c.swap;
  const planes = { h: new THREE.Plane(), v: new THREE.Plane() };
  const both = [planes.h, planes.v];
  let on = false;
  ctx.renderer.localClippingEnabled = true;
  const caps = new THREE.Group();
  caps.visible = false;
  ctx.scene.add(caps);
  const capFor = new Map(); // source mesh -> its stencil and cap meshes
  let made = 0; // render-order slots handed out, so each mesh's stencil pass is its own

  ctx.button.addEventListener('click', () => set(!on));
  panel.addEventListener('input', (e) => {
    const k = e.target.name;
    if (!cut[k]) return;
    cut[k].t = e.target.value / 1000;
    place();
  });
  panel.addEventListener('click', (e) => {
    const k = e.target.closest('[data-swap]')?.dataset.swap;
    if (!k) return;
    e.preventDefault();
    cut[k].swap = !cut[k].swap;
    e.target.classList.toggle('on', cut[k].swap);
    e.target.setAttribute('aria-pressed', cut[k].swap);
    // Keep the view whole until the slider moves: start from the end that cuts nothing.
    cut[k].t = keepsHigh(cut[k]) ? 0 : 1;
    panel.querySelector(`[name=${k}]`).value = cut[k].t * 1000;
    place();
  });

  function set(v) {
    on = v;
    panel.hidden = !on;
    ctx.button.classList.toggle('on', on);
    ctx.button.setAttribute('aria-pressed', on);
    caps.visible = on;
    if (on) place();
  }

  // What's shown now, ignoring the cut.
  function bounds() {
    const b = new THREE.Box3();
    for (const g of ctx.groups) g.traverseVisible((o) => { if (o.isMesh) b.expandByObject(o); });
    return b;
  }

  function place() {
    if (!on) return;
    const b = bounds();
    if (b.isEmpty()) return;
    const s = b.getSize(new THREE.Vector3());
    const a = s.x >= s.y ? 0 : 1;
    const along = new THREE.Vector3().setComponent(a, 1);
    const up = new THREE.Vector3(0, 0, 1);
    // Just past the ends at 0 and 1, so the end positions cut nothing.
    const at = (lo, hi, t) => lo - 0.5 + t * (hi - lo + 1);
    const set1 = (plane, axis, lo, hi, c) => {
      const p = at(lo, hi, c.t);
      // Keep from p up to hi, or from lo up to p.
      if (keepsHigh(c)) plane.set(axis.clone(), -p);
      else plane.set(axis.clone().negate(), p);
    };
    set1(planes.h, along, b.min.getComponent(a), b.max.getComponent(a), cut.h);
    set1(planes.v, up, b.min.z, b.max.z, cut.v);
  }

  // Meshes that are closed solids, so a cut through them can be capped.
  const solid = (o) => o.isMesh && !o.isInstancedMesh && !o.userData.billboard && ['tray', 'label', 'ghost', 'printed_box', 'printed_lid'].includes(o.userData.kind);
  const shown = (o) => { for (let x = o; x; x = x.parent) if (!x.visible) return false; return true; };

  function capsOf(src, k) {
    const mat0 = [].concat(src.material)[0];
    const color = (mat0?.color?.clone() || new THREE.Color(0x999999)).multiplyScalar(0.8);
    const parts = [];
    both.forEach((plane, i) => {
      const base = 1 + (k * 2 + i) * 3;
      const stencil = (side, op) => {
        const m = new THREE.Mesh(src.geometry, new THREE.MeshBasicMaterial({
          side, colorWrite: false, depthWrite: false, depthTest: false, clippingPlanes: [plane],
          stencilWrite: true, stencilFunc: THREE.AlwaysStencilFunc, stencilFail: op, stencilZFail: op, stencilZPass: op,
        }));
        m.renderOrder = base;
        return m;
      };
      const cap = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), new THREE.MeshStandardMaterial({
        color, roughness: 0.8, side: THREE.DoubleSide, clippingPlanes: both.filter((p) => p !== plane),
        stencilWrite: true, stencilRef: 0, stencilFunc: THREE.NotEqualStencilFunc,
        stencilFail: THREE.ReplaceStencilOp, stencilZFail: THREE.ReplaceStencilOp, stencilZPass: THREE.ReplaceStencilOp,
      }));
      cap.renderOrder = base + 1;
      cap.userData.plane = plane;
      for (const m of [stencil(THREE.BackSide, THREE.IncrementWrapStencilOp), stencil(THREE.FrontSide, THREE.DecrementWrapStencilOp)]) {
        m.matrixAutoUpdate = false;
        parts.push(m);
      }
      parts.push(cap);
    });
    caps.add(...parts);
    return parts;
  }

  // Stencils follow their meshes; caps lie on their planes, covering the mesh.
  function sync() {
    const live = new Set();
    for (const g of ctx.groups) g.traverse((o) => { if (solid(o)) live.add(o); });
    for (const [src, parts] of capFor) {
      if (live.has(src)) continue;
      // The stencils share the mesh's geometry; only a cap's own plane is theirs to free.
      for (const p of parts) { caps.remove(p); if (p.userData.plane) p.geometry.dispose(); p.material.dispose(); }
      capFor.delete(src);
    }
    for (const src of live) {
      if (!capFor.has(src)) capFor.set(src, capsOf(src, made++));
      const vis = shown(src);
      src.updateMatrixWorld();
      const sphere = (src.geometry.boundingSphere || (src.geometry.computeBoundingSphere(), src.geometry.boundingSphere)).clone().applyMatrix4(src.matrixWorld);
      for (const p of capFor.get(src)) {
        p.visible = vis;
        if (p.userData.plane) {
          // A square on the plane, centred under the mesh and big enough to cover its section.
          const pl = p.userData.plane;
          const at = pl.projectPoint(sphere.center, new THREE.Vector3());
          p.position.copy(at);
          p.lookAt(at.clone().add(pl.normal));
          p.scale.setScalar(sphere.radius * 2.2);
        } else {
          p.matrix.copy(src.matrixWorld);
        }
      }
    }
  }

  // Every frame: what's shown is clipped while cutting (new meshes from a rebuild too), and its
  // caps follow it.
  function update() {
    for (const g of ctx.groups) {
      g.traverse((o) => {
        for (const m of [].concat(o.material || [])) {
          if (on && m.clippingPlanes !== both) { m.clippingPlanes = both; m.needsUpdate = true; }
          if (!on && m.clippingPlanes === both) { m.clippingPlanes = null; m.needsUpdate = true; }
        }
      });
    }
    if (on) sync();
  }

  return { update, place, set, active: () => on, planes: () => (on ? both : []) };
}
