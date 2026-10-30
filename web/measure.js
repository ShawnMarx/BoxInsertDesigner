// Measuring a tray in the viewer (the Tray tab's Measure button). A tap near a corner places a
// point there; near an edge, the whole straight edge lights up with its length; anywhere else
// on the tray, a point on its surface. A second point gives the distance between the two, and
// a third starts over. Everything is in mm, read off the tray's mesh as built.
//
// With Cut view on, only what the cut leaves counts: taps go through the cut-away part, land on
// the solid cut face as well as the surfaces, and snap to the cut face's outline and corners.

import * as THREE from 'three';

const SNAP_PX = 12; // how near a tap must land to a corner or an edge, in screen pixels
const COLOR = 0xe8590c;

export function initMeasure(ctx) {
  // ctx: { camera, canvas, scene, button, targets: () => meshes to measure, planes: () => cut planes }
  const group = new THREE.Group();
  group.renderOrder = 10;
  ctx.scene.add(group);
  const layer = document.createElement('div');
  layer.className = 'measure-layer';
  ctx.canvas.parentElement.append(layer);
  const hint = document.createElement('div');
  hint.className = 'measure-hint';
  hint.hidden = true;
  ctx.canvas.parentElement.append(hint);
  let on = false;
  let points = []; // world positions
  let labels = []; // { el, at: Vector3 }
  const mat = new THREE.MeshBasicMaterial({ color: COLOR, depthTest: false, transparent: true });

  ctx.button.addEventListener('click', () => set(!on));
  addEventListener('keydown', (e) => { if (on && e.key === 'Escape') clear(); });

  function set(v) {
    on = v;
    ctx.button.classList.toggle('on', on);
    ctx.button.setAttribute('aria-pressed', on);
    hint.hidden = !on;
    hint.textContent = 'Tap an edge for its length, or two points for the distance between them';
    if (!on) clear();
  }

  function clear() {
    points = [];
    group.traverse((o) => { if (o !== group) o.geometry?.dispose(); });
    group.clear();
    for (const l of labels) l.el.remove();
    labels = [];
  }

  // The size of one screen pixel in the world, at a point.
  function worldPerPx(p) {
    const cam = ctx.camera;
    const d = cam.position.distanceTo(p);
    return (2 * d * Math.tan(THREE.MathUtils.degToRad(cam.fov / 2))) / ctx.canvas.clientHeight;
  }

  const cuts = () => ctx.planes?.() || [];
  const kept = (p, but) => cuts().every((pl) => pl === but || pl.distanceToPoint(p) >= -1e-3);

  // A segment's part the cut leaves, or null.
  function clipSeg([a, b]) {
    for (const pl of cuts()) {
      const da = pl.distanceToPoint(a);
      const db = pl.distanceToPoint(b);
      if (da < 0 && db < 0) return null;
      if (da < 0) a = a.clone().lerp(b, da / (da - db));
      else if (db < 0) b = b.clone().lerp(a, db / (db - da));
    }
    return [a, b];
  }

  // Where a plane cuts a mesh: the outline of its cut face, as segments, in world coordinates.
  function section(mesh, pl) {
    const g = mesh.geometry;
    const pos = g.attributes.position;
    const idx = g.index;
    const n = idx ? idx.count : pos.count;
    const v = [new THREE.Vector3(), new THREE.Vector3(), new THREE.Vector3()];
    const d = [0, 0, 0];
    const segs = [];
    for (let i = 0; i < n; i += 3) {
      for (let j = 0; j < 3; j++) {
        v[j].fromBufferAttribute(pos, idx ? idx.getX(i + j) : i + j).applyMatrix4(mesh.matrixWorld);
        d[j] = pl.distanceToPoint(v[j]);
      }
      const cross = [];
      for (let j = 0; j < 3; j++) {
        const k = (j + 1) % 3;
        if ((d[j] < 0) !== (d[k] < 0)) cross.push(v[j].clone().lerp(v[k], d[j] / (d[j] - d[k])));
      }
      if (cross.length === 2 && cross[0].distanceTo(cross[1]) > 1e-4) segs.push(cross);
    }
    return segs;
  }

  // A mesh's sharp edges in world coordinates, cached on the mesh. With a cut: the parts of them
  // it leaves, and the outline of each cut face.
  function edges(mesh) {
    const all = sharp(mesh).map(clipSeg).filter(Boolean);
    for (const pl of cuts()) for (const s of section(mesh, pl)) { const c = clipSeg(s); if (c) all.push(c); }
    return all;
  }

  function sharp(mesh) {
    const u = mesh.userData;
    if (!u.measureEdges) {
      const pos = new THREE.EdgesGeometry(mesh.geometry, 25).attributes.position;
      u.measureEdges = [];
      for (let i = 0; i < pos.count; i += 2) u.measureEdges.push([new THREE.Vector3().fromBufferAttribute(pos, i), new THREE.Vector3().fromBufferAttribute(pos, i + 1)]);
    }
    mesh.updateMatrixWorld();
    return u.measureEdges.map(([a, b]) => [a.clone().applyMatrix4(mesh.matrixWorld), b.clone().applyMatrix4(mesh.matrixWorld)]);
  }

  // The whole straight edge a segment is part of: every segment on its line, joined end to end.
  function fullEdge(seg, all) {
    const [a, b] = seg;
    const dir = b.clone().sub(a).normalize();
    const along = (p) => p.clone().sub(a).dot(dir);
    const offLine = (p) => p.clone().sub(a).sub(dir.clone().multiplyScalar(along(p))).length();
    const spans = all.filter(([p, q]) => offLine(p) < 0.01 && offLine(q) < 0.01).map(([p, q]) => [Math.min(along(p), along(q)), Math.max(along(p), along(q))]);
    spans.sort((x, y) => x[0] - y[0]);
    let [lo, hi] = [0, along(b)];
    for (let grew = true; grew;) {
      grew = false;
      for (const [s, e] of spans) {
        if (s <= hi + 0.01 && e >= lo - 0.01 && (s < lo - 1e-6 || e > hi + 1e-6)) { lo = Math.min(lo, s); hi = Math.max(hi, e); grew = true; }
      }
    }
    return [a.clone().add(dir.clone().multiplyScalar(lo)), a.clone().add(dir.clone().multiplyScalar(hi))];
  }

  function bar(a, b, r) {
    const len = a.distanceTo(b);
    const m = new THREE.Mesh(new THREE.CylinderGeometry(r, r, len, 8), mat);
    m.position.copy(a).add(b).multiplyScalar(0.5);
    m.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), b.clone().sub(a).normalize());
    m.renderOrder = 10;
    group.add(m);
  }

  function dot(p, r) {
    const m = new THREE.Mesh(new THREE.SphereGeometry(r, 16, 12), mat);
    m.position.copy(p);
    m.renderOrder = 10;
    group.add(m);
  }

  const f = (v) => `${(+v).toFixed(1).replace(/\.0$/, '')}`;
  function label(at, text, sub) {
    const el = document.createElement('div');
    el.className = 'measure-label';
    el.innerHTML = `<b>${text}</b>${sub ? `<span>${sub}</span>` : ''}`;
    layer.append(el);
    labels.push({ el, at });
  }

  // A tap (not a drag) on the canvas while measuring. Returns true when it was handled.
  function tap(e) {
    if (!on) return false;
    const r = ctx.canvas.getBoundingClientRect();
    const ray = new THREE.Raycaster();
    ray.setFromCamera(new THREE.Vector2(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1), ctx.camera);
    const meshes = ctx.targets();
    // Both sides, so that crossings in and out of each solid count (for the cut face).
    const sides = meshes.map((m) => [].concat(m.material).map((mt) => [mt, mt.side]));
    for (const s of sides) for (const [mt] of s) mt.side = THREE.DoubleSide;
    const hits = ray.intersectObjects(meshes, false);
    for (const s of sides) for (const [mt, side] of s) mt.side = side;
    // The nearest surface the cut leaves, or a cut face nearer than it: where the ray meets a
    // plane inside a solid (an odd number of crossings before it).
    let hit = hits.find((x) => kept(x.point));
    for (const pl of cuts()) {
      const at = ray.ray.intersectPlane(pl, new THREE.Vector3());
      if (!at || !kept(at, pl)) continue;
      const dist = ray.ray.origin.distanceTo(at);
      const before = hits.filter((x) => x.distance < dist);
      if (before.length % 2 === 1 && (!hit || dist < hit.distance)) hit = { point: at, distance: dist, object: before[before.length - 1].object };
    }
    if (!hit) { clear(); return true; }
    const h = hit.point;
    const scale = worldPerPx(h);
    const tol = SNAP_PX * scale;
    const all = edges(hit.object);
    // Nearest corner, then nearest edge, to where the tap met the tray.
    let corner = null;
    let cd = tol;
    for (const s of all) for (const p of s) { const d = p.distanceTo(h); if (d < cd) { cd = d; corner = p; } }
    let seg = null;
    let sd = tol;
    const line = new THREE.Line3();
    const near = new THREE.Vector3();
    for (const s of all) {
      line.set(s[0], s[1]);
      line.closestPointToPoint(h, true, near);
      const d = near.distanceTo(h);
      if (d < sd) { sd = d; seg = s; }
    }
    const r0 = Math.max(0.25, 2.2 * scale);
    if (!corner && seg) {
      clear();
      const [a, b] = fullEdge(seg, all);
      bar(a, b, r0);
      dot(a, r0 * 1.8);
      dot(b, r0 * 1.8);
      label(a.clone().add(b).multiplyScalar(0.5), `${f(a.distanceTo(b))} mm`, 'edge');
      return true;
    }
    if (points.length >= 2 || labels.some((l) => l.el.querySelector('span')?.textContent === 'edge')) clear();
    const p = (corner || h).clone();
    points.push(p);
    dot(p, r0 * 1.8);
    if (points.length === 2) {
      const [a, b] = points;
      bar(a, b, r0);
      const d = b.clone().sub(a);
      const parts = [['x', d.x], ['y', d.y], ['z', d.z]].filter(([, v]) => Math.abs(v) >= 0.05).map(([k, v]) => `${k} ${f(Math.abs(v))}`);
      label(a.clone().add(b).multiplyScalar(0.5), `${f(a.distanceTo(b))} mm`, parts.length > 1 ? parts.join(' · ') : '');
    }
    return true;
  }

  // Every frame: labels follow their points on screen.
  function update() {
    if (!labels.length) return;
    const w = ctx.canvas.clientWidth;
    const h = ctx.canvas.clientHeight;
    for (const l of labels) {
      const p = l.at.clone().project(ctx.camera);
      l.el.style.transform = `translate(${((p.x + 1) / 2) * w}px, ${((1 - p.y) / 2) * h}px) translate(-50%, -130%)`;
      l.el.hidden = p.z > 1;
    }
  }

  return { tap, update, clear, set, active: () => on };
}
