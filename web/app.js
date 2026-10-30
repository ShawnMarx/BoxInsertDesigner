// Box Insert Designer front end. A view of the spec on the server: it reads /api/games/<slug>/state,
// loads the GLB SolidCraft built, and follows /api/events so every change shows up without a
// reload. Actions go through the same command API as the `bid` CLI.

import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { initMotion } from './motion.js';
import { initMeasure } from './measure.js';
import { initCut } from './cut.js';
import { initPlates } from './plates.js';
import { initSolidcraft, openEditor, openPiece, editorStatus, noteFrom } from './solidcraft.js';
import { initComponents, renderComponents, describeMeasure, editComponent, newComponent, slugify } from './components.js';

const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const mm = (v) => `${(+v).toFixed(1).replace(/\.0$/, '')}`;
const dims = (a) => a.map(mm).join(' × ') + ' mm';

// The game shown: ?game=<slug> pins it; otherwise follow the server's current game.
const params = new URLSearchParams(location.search);
const pinned = params.get('game');
let game = pinned;
let follow = !pinned;
let state = null;
let selected = null;
let glbVersion = null;
let fitted = false;
// The box shown in the Insert tab (?box=), and the tab (?tab=).
let boxId = params.get('box');
let tab = params.get('tab') || 'insert';
// The tray the Tray tab is about (?tray=).
let trayTab = params.get('tray');
// A component looked at on its own ('alone') or picked out in its box ('box').
let focus = null;
const setParam = (k, v) => { const u = new URL(location); if (v) u.searchParams.set(k, v); else u.searchParams.delete(k); history.replaceState(null, '', u); };
const curBox = () => state?.boxes?.find((b) => b.id === boxId) || state?.boxes?.[0] || null;

// ---------- three.js ----------
const canvas = $('view');
const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true, stencil: true });
renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera(35, 1, 1, 20000);
camera.up.set(0, 0, 1);
const controls = new OrbitControls(camera, canvas);
controls.enableDamping = true;
scene.add(new THREE.HemisphereLight(0xffffff, 0x8a8778, 1.6));
const sun = new THREE.DirectionalLight(0xffffff, 1.6);
sun.position.set(-300, -500, 900);
scene.add(sun);

const boxGroup = new THREE.Group();
const modelGroup = new THREE.Group();
const soloGroup = new THREE.Group();
// As-is components placed in the box (boards between layers), as slabs.
const boardGroup = new THREE.Group();
// The outlines of what's around a tray picked out in its box, or above the layer slider.
const frameGroup = new THREE.Group();
// A bin picked out on the Tray tab.
const pickGroup = new THREE.Group();
scene.add(boxGroup, modelGroup, soloGroup, boardGroup, frameGroup, pickGroup);
const loader = new GLTFLoader();

const view = { ghosts: true, box: true, lift: false, upTo: Infinity, art: false, lid: false, around: 'pulse' };
try { view.around = localStorage.getItem('bid.view.around') || 'pulse'; } catch {}

function resize() {
  const r = canvas.parentElement.getBoundingClientRect();
  renderer.setSize(r.width, r.height, false);
  camera.aspect = r.width / Math.max(r.height, 1);
  camera.updateProjectionMatrix();
}
new ResizeObserver(resize).observe(canvas.parentElement);
// The motion video and the Tray tab's measuring, once the viewer is set up (below).
let motion = null;
let measure = null;
let plates = null;
let cut = null;
renderer.setAnimationLoop((now) => {
  controls.update();
  motion?.update(now);
  measure?.update();
  cut?.update();
  tickAround(now);
  for (const m of modelGroup.children) {
    if (m.userData.billboard) m.rotation.z = Math.atan2(camera.position.y - m.position.y, camera.position.x - m.position.x) - Math.PI / 2;
  }
  renderer.render(scene, camera);
});

function disposeGroup(g) {
  g.traverse((o) => { o.geometry?.dispose(); edgeCache.get(o)?.geometry.dispose(); if (o.material) [].concat(o.material).forEach((m) => m.dispose()); });
  g.clear();
}

function drawBox(box, layout) {
  // The motion video owns the box while it plays, and redraws it when it stops.
  if (motion?.playing()) return;
  disposeGroup(boxGroup);
  const [L, W, H] = box.size.inner;
  // A printed box is in the model itself; a bought one is drawn as its inside.
  if (!box.printed) {
    const geo = new THREE.BoxGeometry(L, W, H);
    geo.translate(L / 2, W / 2, H / 2);
    const shell = new THREE.Mesh(geo, new THREE.MeshStandardMaterial({ color: 0x9aa4b1, transparent: true, opacity: 0.08, depthWrite: false, side: THREE.BackSide }));
    const edges = new THREE.LineSegments(new THREE.EdgesGeometry(geo), new THREE.LineBasicMaterial({ color: 0x6f7885 }));
    boxGroup.add(shell, edges);
  }
  // The room the top stack (boards, rules) takes, under the lid.
  const avail = layout?.report.available_height.mm ?? H;
  const top = H + (box.size.lid_depth || 0) + (layout?.report.lid_lift?.mm || 0);
  if (top - avail > 0.5) {
    const g2 = new THREE.BoxGeometry(L, W, top - avail);
    g2.translate(L / 2, W / 2, avail + (top - avail) / 2);
    const fill = new THREE.Mesh(g2, new THREE.MeshStandardMaterial({ color: 0xc9a86a, transparent: true, opacity: 0.12, depthWrite: false }));
    const e2 = new THREE.LineSegments(new THREE.EdgesGeometry(g2), new THREE.LineBasicMaterial({ color: 0xb08a45, transparent: true, opacity: 0.6 }));
    fill.name = 'top-stack';
    e2.name = 'top-stack-edges';
    boxGroup.add(fill, e2);
  }
  // What goes in as it is (boards, rules): slabs in the top stack, from the measurements.
  let z = avail;
  // Not what the layout already places on top: the model draws that, where the packer put it,
  // and a second copy in the same place flickers against it (z-fighting).
  const onTop = new Set((layout?.on_top || []).map((o) => o.component));
  for (const c of asIsIn(box.id).filter((x) => !onTop.has(x.id))) {
    const d = mainDims(c);
    if (!d) continue;
    const [w, dd, h] = d;
    const g = new THREE.BoxGeometry(Math.min(w, L), Math.min(dd, W), h);
    g.translate(L / 2, W / 2, z + h / 2);
    const tex = state.textures?.[c.id]?.top;
    // Solid as it lies; the slider's step below it shows the insert under it.
    const slab = new THREE.Mesh(g, [0, 1, 2, 3, 4, 5].map((i) => new THREE.MeshStandardMaterial({
      ...(i === 4 && tex ? { map: texFor(tex) } : { color: 0xb9a57a }), transparent: true, opacity: 0.3, depthWrite: false })));
    slab.userData = { kind: 'as_is', component: c.id };
    slab.name = 'top-stack-item';
    boxGroup.add(slab);
    z += h;
  }
  // As-is components placed in the box (a board on a layer), where they lie.
  disposeGroup(boardGroup);
  for (const c of (state.spec.components || []).filter((x) => x.kind === 'as_is' && x.place && (x.box || state.boxes[0]?.id) === box.id)) {
    const d = mainDims(c);
    if (!d) continue;
    const [w, dd, h] = c.place.turn % 180 ? [d[1], d[0], d[2]] : d;
    const g = new THREE.BoxGeometry(w, dd, h);
    const [x, y, zz] = c.place.at;
    g.translate(w / 2, dd / 2, h / 2);
    const tex = state.textures?.[c.id]?.top;
    const slab = new THREE.Mesh(g, [0, 1, 2, 3, 4, 5].map((i) => new THREE.MeshStandardMaterial({ ...(i === 4 && tex ? { map: texFor(tex) } : { color: 0xc9b98f }), roughness: 0.8 })));
    slab.position.set(x, y, zz);
    slab.userData = { kind: 'board', component: c.id, layer: (c.place.layer || 1) - 1, baseZ: zz, name: c.name };
    boardGroup.add(slab);
  }
  // Box art: a plain base around the insert and the telescoping lid lifted above it, with the
  // lid's photos on its top and skirt.
  const bc = state.spec.components.find((x) => x.id === box.id);
  if (view.art && bc && !box.printed) {
    const g = boxShapes(bc, L >= W);
    const base = new THREE.Mesh(g.base, g.baseMats);
    base.position.set(L / 2, W / 2, g.baseZ);
    const lid = new THREE.Mesh(g.lid, g.lidMats);
    lid.position.set(L / 2, W / 2, top + Math.max(40, H * 0.6) + g.skirt / 2);
    base.name = lid.name = 'box-art';
    boxGroup.add(base, lid);
  }
  boxGroup.visible = view.box;
}

function asIsIn(id) {
  return (state.spec.components || []).filter((c) => c.kind === 'as_is' && !c.place && !c.piece && (c.box || state.boxes[0]?.id) === id);
}

// The main measurement as [w, d, h] mm, else the packer's fields.
function mainDims(c) {
  const m = c.measure;
  if (m) {
    const d = m.of === 'stack' ? (m.stack || m.piece) : (m.piece || m.stack);
    if (d) return d;
  }
  if (c.kind === 'cards') return [c.size[0], c.size[1], c.stack || (c.count || 1) * 0.31];
  if (c.kind === 'tokens') { const w = c.size[0], d = c.size[1] ?? w; return [w, d, c.thickness * c.count]; }
  if (c.kind === 'mini') return [c.base, c.base, c.height];
  if (c.kind === 'bin') return [c.size[0], c.size[1], c.depth];
  if (c.kind === 'box') return c.inner;
  return null;
}

// A box's outside: measured, else the inside with a nominal 2 mm board all round.
function outside(c) {
  const m = c.measure?.piece || c.measure?.stack;
  const d = m || [c.inner[0] + 4, c.inner[1] + 4, c.inner[2] + 2];
  return [Math.max(d[0], d[1]), Math.min(d[0], d[1]), d[2]];
}

const BOARD = 1.5;

// A game box as two shapes: the base (open top, plain board) and the telescoping lid (open
// bottom: its art on top, its front and side photos on the skirt). Sizes from the measured
// outside (the closed box, so the lid), else the inside plus nominal board; `landscape` turns
// them to match the inside's orientation. Base geometry is centred; baseZ puts its floor at 0.
function boxShapes(c, landscape) {
  const [oL, oW, oH] = outside(c);
  const [lx, ly] = landscape ? [oL, oW] : [oW, oL];
  const skirt = Math.min(c.lid_height || oH, oH);
  const faces = state.textures?.[c.id] || {};
  const art = (face) => new THREE.MeshStandardMaterial(faces[face] ? { map: texFor(faces[face]), side: THREE.DoubleSide } : { color: 0x8c8574, side: THREE.DoubleSide });
  const plain = () => new THREE.MeshStandardMaterial({ color: 0xb3aa96, side: THREE.DoubleSide });
  const hidden = () => new THREE.MeshStandardMaterial({ visible: false });
  // Face order: +x, −x, +y, −y, +z, −z. The front photo goes on the long sides.
  const longIsY = lx >= ly;
  const sideMat = (axis) => art((axis === 'y') === longIsY ? 'front' : 'side');
  const baseH = oH - BOARD;
  const floor = Math.max(baseH - c.inner[2], 0);
  return {
    base: new THREE.BoxGeometry(lx - 2 * BOARD, ly - 2 * BOARD, baseH),
    baseMats: [plain(), plain(), plain(), plain(), hidden(), plain()],
    baseZ: baseH / 2 - floor,
    lid: uprightUVs(new THREE.BoxGeometry(lx, ly, skirt)),
    lidMats: [sideMat('x'), sideMat('x'), sideMat('y'), sideMat('y'), art('lid'), hidden()],
    skirt, oH, floor, size: [lx, ly, oH],
  };
}

// Box faces with each photo upright and unmirrored, seen from outside: three.js lays the ±x
// faces' images on their side. The top keeps its image's top towards +y.
function uprightUVs(geo) {
  const p = geo.attributes.position, n = geo.attributes.normal, uv = geo.attributes.uv;
  geo.computeBoundingBox();
  const { min, max } = geo.boundingBox;
  const f = (v, a, b) => (v - a) / (b - a);
  for (let i = 0; i < p.count; i++) {
    const [x, y, z] = [p.getX(i), p.getY(i), p.getZ(i)];
    const [nx, ny, nz] = [n.getX(i), n.getY(i), n.getZ(i)];
    const vz = f(z, min.z, max.z);
    if (nx > 0.5) uv.setXY(i, f(y, min.y, max.y), vz);
    else if (nx < -0.5) uv.setXY(i, f(y, max.y, min.y), vz);
    else if (ny > 0.5) uv.setXY(i, f(x, max.x, min.x), vz);
    else if (ny < -0.5) uv.setXY(i, f(x, min.x, max.x), vz);
    else if (nz > 0.5) uv.setXY(i, f(x, min.x, max.x), f(y, min.y, max.y));
    else uv.setXY(i, f(x, min.x, max.x), f(y, max.y, min.y));
  }
  uv.needsUpdate = true;
  return geo;
}

function fitCamera(size, force) {
  if (fitted && !force) return;
  fitted = true;
  // An empty printed box has no size yet.
  const [L, W, H] = size.map((v) => v || 100);
  const c = new THREE.Vector3(L / 2, W / 2, H / 3);
  const r = Math.hypot(L, W, H);
  camera.position.set(c.x - r * 0.55, c.y - r * 1.05, c.z + r * 0.95);
  controls.target.copy(c);
  camera.near = r / 100; camera.far = r * 20; camera.updateProjectionMatrix();
}

async function loadGlb(url) {
  const gltf = await loader.loadAsync(url);
  motion?.stop();
  measure?.clear();
  disposeGroup(modelGroup);
  gltf.scene.traverse((o) => {
    if (!o.isMesh) return;
    const ex = o.userData || {};
    o.userData = { ...ex };
    if (ex.kind === 'ghost') {
      o.material.transparent = true;
      o.material.opacity = 0.42;
      o.material.depthWrite = false;
      o.material.side = THREE.FrontSide;
      o.renderOrder = 2;
    }
  });
  // Lift the meshes out of the glTF scene graph so each keeps its own userData at top level.
  const meshes = [];
  gltf.scene.traverse((o) => { if (o.isMesh) meshes.push(o); });
  for (const m of meshes) { m.updateWorldMatrix(true, false); m.applyMatrix4(m.parent.matrixWorld); m.removeFromParent(); modelGroup.add(m); m.userData.baseZ = m.position.z; }
  dressedKey = null;
  dressGhosts();
  applyView();
}

function applyView() {
  const alone = focus?.mode === 'alone';
  soloGroup.visible = alone;
  modelGroup.visible = !alone;
  boxGroup.visible = view.box && !alone;
  const nLayers = state?.layout?.layers.length ?? 1;
  // A tray on its own: just it, and (with Contents on) what it holds.
  const onlyTray = focus?.mode === 'tray' ? focus.id : null;
  if (onlyTray) boxGroup.visible = false;
  // A tray picked out in its box: it stays solid with its contents; the other trays fade.
  const trayInBox = focus?.mode === 'tray-box' ? focus.id : null;
  // Picked out in its box: its own contents and trays stay, everything else fades.
  const inBox = focus?.mode === 'box' ? focus.id : null;
  const focusTrays = new Set(inBox ? modelGroup.children.filter((m) => m.userData.component === inBox).map((m) => m.userData.tray) : []);
  // What's around the tray picked out in its box, and what's above the layer slider, is the
  // insert's context: it pulses, shows as an outline, or hides (view.around).
  const ctxOn = !alone && !onlyTray && !inBox && !motion?.playing();
  around.list = [];
  const toCtx = (m) => { m.userData.ctx = true; m.visible = false; around.list.push(m); };
  for (const m of modelGroup.children) {
    const u = m.userData;
    const layer = u.layer ?? 0;
    const mine = !!trayInBox && u.tray === trayInBox;
    u.ctx = false;
    m.visible = !u.replaced && (layer < view.upTo || mine) && (u.kind !== 'ghost' || view.ghosts || u.component === inBox);
    if (inBox && u.kind === 'ghost' && u.component !== inBox) m.visible = false;
    if (onlyTray) m.visible = u.tray === onlyTray && !u.replaced && (u.kind !== 'ghost' || view.ghosts);
    // A printed box goes with the Box toggle; its lid shows only when asked for, closed.
    if (u.kind === 'printed_box') m.visible = view.box && !onlyTray && !inBox && !trayInBox;
    if (u.kind === 'printed_lid') m.visible = view.box && view.lid && !onlyTray && !inBox && !trayInBox && !view.lift && view.upTo >= nLayers;
    if (ctxOn && !u.replaced && !mine && (u.kind === 'tray' || u.kind === 'label' || (u.kind === 'ghost' && view.ghosts)) && (trayInBox || layer >= view.upTo)) toCtx(m);
    m.position.z = u.baseZ + (view.lift && !onlyTray ? layer * 40 : 0);
    // An inlaid label goes with its tray.
    if (u.kind === 'tray' || u.kind === 'label') {
      for (const mat of [].concat(m.material)) {
        // The trays holding it go see-through so its contents show; the rest nearly vanish.
        if (!u.ctx) { if (mat.transparent !== !!inBox) mat.needsUpdate = true; mat.transparent = !!inBox; mat.opacity = !inBox ? 1 : focusTrays.has(u.tray) ? 0.3 : 0.08; mat.depthWrite = !inBox; }
        mat.emissive?.setHex(selected && u.tray === selected ? 0x333333 : 0x000000);
      }
    }
    if (u.kind === 'ghost') {
      if (!u.ctx) unfade(m);
      for (const mat of [].concat(m.material)) mat.emissive?.setHex(inBox && u.component === inBox ? 0x664400 : 0x000000);
    }
  }
  for (const m of boardGroup.children) {
    const u = m.userData;
    u.ctx = false;
    m.visible = !alone && !onlyTray && view.ghosts && u.layer < view.upTo && !trayInBox && (!inBox || u.component === inBox);
    if (ctxOn && view.ghosts && (trayInBox || u.layer >= view.upTo)) toCtx(m); else unfade(m);
    m.position.z = u.baseZ + (view.lift ? u.layer * 40 : 0);
  }
  for (const m of boxGroup.children) {
    if (m.name === 'top-stack-item') {
      m.userData.ctx = false;
      // What lies on top (a board, the rules) is the layer above the trays' (the slider's last
      // step), and contents: it goes with the Contents toggle, solid like a board on a layer.
      m.visible = !view.lift && view.upTo > nLayers && !trayInBox && (view.ghosts || inBox === m.userData.component) && (!inBox || m.userData.component === inBox);
      if (ctxOn && view.ghosts && !view.lift && (trayInBox || view.upTo <= nLayers)) toCtx(m);
      else for (const mat of m.material) {
        const see = !!inBox && inBox !== m.userData.component;
        if (mat.transparent !== see) mat.needsUpdate = true;
        mat.opacity = see ? 0.3 : 1; mat.transparent = see; mat.depthWrite = !see;
      }
    }
  }
  // The top stack's band (and its outline) only with the whole insert in view: a tray or a
  // component picked out, or layers hidden, leaves nothing standing in for the board.
  for (const n of ['top-stack', 'top-stack-edges']) {
    const top = boxGroup.getObjectByName(n);
    if (top) top.visible = view.upTo > nLayers && !view.lift && !trayInBox && !inBox;
  }
  paintAround();
  // The cut's range follows what's shown.
  cut?.place();
}

// Around a tray picked out in its box, the rest of the insert pulses (fades at once, comes back
// once, then stays away), leaving the tray and the box's frame; moving or zooming pulses it again. 'frame' keeps it
// as outlines instead, 'shade' leaves it faintly there, and 'hide' just hides it. Layers above the slider are treated the same.
const around = { list: [], alpha: 0, want: false, start: -1, keys: null, fresh: false };
const AROUND_PEAK = 0.22;
const AROUND_SHADE = 0.08;
const edgeCache = new WeakMap();
const frameMat = new THREE.LineBasicMaterial({ color: 0x6f7885, transparent: true, opacity: 0.5, depthWrite: false });
// One pulse: shown, fading at once, back once, then gone. [ms, alpha] keyframes. Once it has
// gone, a move pulses it back in and out again.
const PULSE = [[0, 1], [700, 0], [1200, 1], [2400, 0]];
const REPULSE = [[0, 0], [500, 1], [1700, 0]];
function pulseAt(t, keys) {
  for (let i = 1; i < keys.length; i++) {
    const [t1, a1] = keys[i], [t0, a0] = keys[i - 1];
    if (t < t1) return a0 + (a1 - a0) * (t - t0) / (t1 - t0);
  }
  return 0;
}
// It starts on the next frame, so a slow redraw doesn't eat it; one already running isn't restarted.
// fresh: a new view (a tray picked, the setting changed), which starts shown.
function pulseAround(fresh = false) {
  if (view.around !== 'pulse' || !around.list.length) return;
  around.want = true;
  if (fresh) { around.fresh = true; around.start = -1; }
}
function tickAround(now) {
  if (view.around !== 'pulse' || !around.list.length) { around.want = false; return; }
  if (around.want && around.start < 0) { around.start = now; around.keys = around.fresh || around.alpha > 0.01 ? PULSE : REPULSE; around.fresh = false; }
  around.want = false;
  if (around.start < 0) return;
  const t = now - around.start;
  const keys = around.keys;
  if (t >= keys[keys.length - 1][0]) around.start = -1;
  const a = pulseAt(t, keys);
  if (a === around.alpha) return;
  around.alpha = a;
  paintAround();
}
// A context mesh's own look, kept the first time it fades, so it can come back as it was.
function unfade(m) {
  for (const mat of [].concat(m.material)) {
    const o = mat.userData.look;
    if (o) { if (mat.transparent !== o[1]) mat.needsUpdate = true; [mat.opacity, mat.transparent, mat.depthWrite] = o; delete mat.userData.look; }
  }
}
function paintAround() {
  frameGroup.clear();
  const a = around.alpha * around.alpha * (3 - 2 * around.alpha);
  for (const m of around.list) {
    // Shade: the old look, the rest left faintly there for good (its contents hidden).
    const shade = view.around === 'shade';
    if (view.around === 'pulse' || (shade && m.userData.kind !== 'ghost')) {
      m.visible = shade || a > 0.01;
      for (const mat of [].concat(m.material)) {
        mat.userData.look ??= [mat.opacity, mat.transparent, mat.depthWrite];
        // Turning see-through recompiles (three.js drops alpha in an opaque material's shader).
        if (!mat.transparent) { mat.transparent = true; mat.needsUpdate = true; }
        mat.depthWrite = false; mat.opacity = shade ? AROUND_SHADE : AROUND_PEAK * a;
      }
    } else if (view.around === 'frame' && m.userData.kind !== 'ghost' && !m.isInstancedMesh) {
      let e = edgeCache.get(m);
      if (!e) {
        // Its bounding box, not every edge: a few dozen trays' worth of edges is a tangle.
        m.geometry.computeBoundingBox();
        const bb = m.geometry.boundingBox, sz = bb.getSize(new THREE.Vector3());
        const g = new THREE.BoxGeometry(sz.x, sz.y, sz.z);
        g.translate(...bb.getCenter(new THREE.Vector3()).toArray());
        e = new THREE.LineSegments(new THREE.EdgesGeometry(g), frameMat);
        g.dispose();
        e.matrixAutoUpdate = false;
        edgeCache.set(m, e);
      }
      m.updateMatrix();
      e.matrix.copy(m.matrix);
      frameGroup.add(e);
    }
  }
}
controls.addEventListener('start', () => pulseAround());
function setAround(mode) {
  view.around = mode;
  try { localStorage.setItem('bid.view.around', mode); } catch {}
  const b = $('t-around');
  b.textContent = `Around: ${{ pulse: 'Pulse', frame: 'Frame', shade: 'Shade', hide: 'Hide' }[mode]}`;
  b.classList.toggle('on', mode !== 'hide');
  around.alpha = 0;
  applyView();
  pulseAround(true);
}

// Tap a tray to select it (ignore drags).
let down = null;
canvas.addEventListener('pointerdown', (e) => { down = [e.clientX, e.clientY]; });
canvas.addEventListener('pointerup', (e) => {
  if (!down || Math.hypot(e.clientX - down[0], e.clientY - down[1]) > 6) return;
  if (measure?.tap(e)) return;
  const r = canvas.getBoundingClientRect();
  const p = new THREE.Vector2(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1);
  const ray = new THREE.Raycaster();
  ray.setFromCamera(p, camera);
  if (focus?.mode === 'alone') return;
  // A tray that is only context (pulsing, shaded) is passed through to the solid one under it;
  // it is picked only when nothing solid is there.
  const hits = ray.intersectObjects(modelGroup.children.filter((m) => m.visible && m.userData.kind === 'tray'));
  const hit = hits.find((h) => !h.object.userData.ctx) || hits[0];
  select(hit ? hit.object.userData.tray : null, true);
});

function select(tray, scroll) {
  selected = selected === tray ? null : tray;
  applyView();
  renderTrays();
  if (selected && scroll) document.querySelector(`#placement [data-tray="${CSS.escape(selected)}"]`)?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
}

// ---------- view controls ----------
function toggle(id, key) {
  const b = $(id);
  b.addEventListener('click', () => {
    view[key] = !view[key];
    b.classList.toggle('on', view[key]);
    b.setAttribute('aria-pressed', view[key]);
    applyView();
  });
}
toggle('t-ghosts', 'ghosts');
toggle('t-box', 'box');
toggle('t-lift', 'lift');
toggle('t-lid', 'lid');
$('t-around').addEventListener('click', () => setAround({ pulse: 'frame', frame: 'shade', shade: 'hide', hide: 'pulse' }[view.around] || 'pulse'));
setAround(view.around);
motion = initMotion({
  scene, camera, controls, renderer, canvas, modelGroup, boxGroup, boxShapes, drawBox, applyView, resize, curBox,
  getState: () => state, game: () => game, button: $('t-motion'),
  onlyTray: () => (tab === 'tray' ? trayTab : null),
});
measure = initMeasure({
  camera, canvas, scene, button: $('t-measure'),
  targets: () => modelGroup.children.filter((m) => m.visible && m.userData.kind === 'tray' && m.userData.tray === trayTab),
  planes: () => cut?.planes() || [],
});
cut = initCut({ renderer, scene, button: $('t-cut'), groups: [modelGroup, boxGroup, soloGroup], viewer: canvas.parentElement });
// Motion moves the pieces about: no measuring or cutting while it's open.
$('t-motion').addEventListener('click', () => { measure.set(false); cut.set(false); });
$('t-art').addEventListener('click', () => {
  view.art = !view.art;
  $('t-art').classList.toggle('on', view.art);
  $('t-art').setAttribute('aria-pressed', view.art);
  if (state?.layout) drawBox(curBox(), state.layout);
  applyView();
});
$('t-reset').addEventListener('click', () => {
  if (focus?.mode === 'alone') fitSolo();
  else if (curBox()) fitCamera(curBox().size.inner, true);
});
$('layer').addEventListener('input', (e) => {
  const n = +e.target.value, max = +e.target.max;
  view.upTo = n >= max ? Infinity : n;
  $('layer-out').textContent = n >= max ? 'all' : `1–${n}`;
  applyView();
  pulseAround();
});

// ---------- commands ----------
async function command(body) {
  const r = await fetch('/api/command', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ game, ...body }) });
  if (r.status >= 500) { toast(`server error ${r.status}`); return { ok: false }; }
  const v = await r.json();
  if (!v.ok) toast(v.reason?.message || 'command failed');
  return v;
}

let toastTimer;
function toast(msg) {
  const t = $('toast');
  t.textContent = msg; t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { t.hidden = true; }, 4000);
}

async function runExport() {
  const v = await command({ cmd: 'export' });
  if (v.ok) showExport(v.data.dir, v.data.files.map((f) => f.file));
  return v;
}
// The files are written on the server; this hands them to the browser.
const outUrl = (f) => `/api/games/${encodeURIComponent(game)}/out/${encodeURIComponent(f)}`;
const zipUrl = () => `/api/games/${encodeURIComponent(game)}/out.zip`;
const projectOf = (files) => files.find((f) => f.endsWith('-plates.3mf'));
function download(url) {
  const a = document.createElement('a');
  a.href = url; a.download = '';
  document.body.append(a); a.click(); a.remove();
}
// Export, then hand the browser the file: for Bambu Studio the project (every piece on its
// plate), else everything, zipped. Every export button goes through here.
async function exportAndDownload(bambu) {
  const v = await runExport();
  if (!v.ok) return v;
  const project = projectOf(v.data.files.map((f) => f.file));
  download(bambu && project ? outUrl(project) : zipUrl());
  toast(bambu && project ? `Downloading ${project}` : 'Downloading every exported file, zipped');
  return v;
}
for (const id of ['export', 'export-3mf']) $(id).addEventListener('click', async () => {
  const b = $(id), was = b.textContent;
  b.disabled = true; b.textContent = 'Exporting…';
  await exportAndDownload(id === 'export-3mf');
  b.disabled = false; b.textContent = was;
});

function showExport(dir, files) {
  const project = projectOf(files);
  // Bambu Studio's link handler fetches the file itself, from this server.
  const bambu = project ? `bambustudio://open?file=${encodeURIComponent(new URL(outUrl(project), location.href).href)}` : '';
  $('export-out').innerHTML = `<p>Exported ${files.length} files at ${new Date().toLocaleTimeString()}.</p>
    <div class="row">${project ? `<a class="button primary" href="${outUrl(project)}" download>Download Bambu project</a>
      <a class="button" href="${esc(bambu)}">Open in Bambu Studio</a>` : ''}
      <a class="button" href="${zipUrl()}" download>Download all (zip)</a></div>
    <details><summary class="small">Each file <span class="muted">— kept on the server in <code>${esc(dir)}</code></span></summary>
      <ul>${files.map((f) => `<li><a href="${outUrl(f)}" download><code>${esc(f)}</code></a></li>`).join('')}</ul></details>`;
}

// ---------- panels ----------
function render() {
  const s = state.spec;
  renderTabs();
  if (!s) {
    $('fit').innerHTML = `<h2>Spec error</h2><p class="bad">${esc(state.spec_error?.message)}</p>`;
    return;
  }
  document.title = `${s.game.name} · Box Insert Designer`;
  $('game-name').textContent = s.game.name;
  $('sample-badge').hidden = !s.game.sample;
  $('tab-comp-count').textContent = `(${s.components.length})`;
  renderComponents();
  if (sheetFor) renderSheet();
  renderBoxTabs();
  renderPlacement();
  renderBoxes();
  if (focus && !focus.mode.startsWith('tray') && !s.components.some((c) => c.id === focus.id)) setFocus(null);

  const B = curBox();
  $('no-box').hidden = !!B;
  for (const id of ['fit', 'variants']) $(id).closest('.card').hidden = !B;
  $('export').closest('.card').hidden = !B;
  $('view-note').hidden = true;
  if (!B) {
    state.layout = null;
    $('no-box').innerHTML = `<h2>No box yet</h2><p class="small">Measure the inside of the game's box to start the insert, or have a box designed to print, sized to what goes in it. The box is a component too: it can have photos of the box and lid. Other components can be added before or after.</p>
      <button class="primary" id="no-box-add">Add the box</button>`;
    $('no-box-add').addEventListener('click', () => editBox(null));
    disposeGroup(boxGroup); disposeGroup(modelGroup);
    renderStatus();
    return;
  }
  state.layout = B.layouts.find((l) => l.strategy === B.chosen) || B.layouts[0];
  const L = state.layout;
  if ((focus?.mode === 'tray' || focus?.mode === 'tray-box') && !L.trays.some((t) => t.id === focus.id)) { focus = null; $('focus-bar').hidden = true; }
  if (tab === 'tray' && focus?.id !== trayTab && L.trays.some((t) => t.id === trayTab)) setFocus({ id: trayTab, mode: 'tray' });

  // Fit report.
  const r = L.report;
  const clr = r.lid_clearance.mm;
  const asIs = asIsIn(B.id);
  const asIsH = asIs.reduce((t, c) => t + (mainDims(c)?.[2] || 0), 0);
  const topWarn = asIs.length && asIsH > B.size.top_stack + 0.5
    ? [`<li class="warn">What goes in as-is is ${mm(asIsH)} mm thick, but the box's top stack is ${mm(B.size.top_stack)} mm: raise the top stack in Boxes</li>`] : [];
  const issues = [...L.problems.map((p) => `<li class="bad">${esc(p.message)}</li>`), ...L.warnings.map((w) => `<li class="warn">${esc(w.message)}</li>`),
    ...(B.build?.warnings || []).map((w) => `<li class="warn">${esc(w.message)}</li>`), ...topWarn];
  // Folded unless something's wrong: it doesn't fit, the lid won't close, a tray is too big for the bed.
  const trouble = !L.ok || L.problems.length > 0 || clr < 0 || !r.bed_fit;
  $('fit').dataset.fold = trouble ? 'open' : 'closed';
  $('fit').innerHTML = `
    <h2>Fit report <span class="muted small">· ${esc(B.name)} · ${trouble ? '<span class="bad">needs attention</span>' : `fits${issues.length ? `, ${issues.length} note${issues.length === 1 ? '' : 's'}` : ''}`}</span></h2>
    <div class="fit-grid">
      <div class="stat" title="${esc(r.lid_clearance.why)}"><b class="${clr >= 0 ? 'good' : 'bad'}">${clr >= 0 ? '+' : ''}${mm(clr)} mm</b><span>under the lid</span></div>
      <div class="stat" title="${esc(r.stack_height.why)} of ${esc(r.available_height.why)}"><b>${mm(r.stack_height.mm)} / ${mm(r.available_height.mm)}</b><span>stack / room, mm</span></div>
      <div class="stat"><b class="${r.bed_fit ? 'good' : 'bad'}">${r.bed_fit ? 'Fits' : 'Too big'}</b><span>X1C bed, every tray</span></div>
      <div class="stat"><b>${r.tray_count} tray${r.tray_count === 1 ? '' : 's'}</b><span>${r.layer_count} layer${r.layer_count === 1 ? '' : 's'}</span></div>
      ${B.size.lid_lift ? `<div class="stat" title="${esc(r.lid_lift.why)}"><b class="${r.lid_lift.mm > 0 ? 'warn' : 'good'}">${r.lid_lift.mm > 0 ? `${mm(r.lid_lift.mm)} mm` : 'None'}</b><span>lid lift, of ${mm(B.size.lid_lift)} allowed</span></div>` : ''}
    </div>
    ${issues.length ? `<ul class="issues">${issues.join('')}</ul>` : ''}
    ${r.unconfirmed.length ? `<details class="small" style="margin-top:8px"><summary><span class="badge um">TO CONFIRM</span> ${r.unconfirmed.length} values are proposals or estimates</summary><ul class="issues">${r.unconfirmed.map((u) => `<li>${esc(u)}</li>`).join('')}</ul></details>` : ''}`;

  // Variants: the one in use, and others only when they fit and lay it out differently.
  const shape = (l) => JSON.stringify(l.trays.map((t) => [t.size, t.pos]).sort());
  const seen = new Set([shape(state.layout)]);
  const variants = B.layouts.filter((l) => {
    if (l.strategy === B.chosen) return true;
    // An empty one (a game of imported pieces, where the packer has nothing to lay out) isn't a choice.
    if (!l.ok || !l.trays.length || seen.has(shape(l))) return false;
    seen.add(shape(l));
    return true;
  });
  $('variants').closest('.card').hidden = variants.length < 2;
  $('variants').innerHTML = variants.map((l) => `
    <button class="variant ${l.strategy === B.chosen ? 'chosen' : ''}" data-strategy="${esc(l.strategy)}">
      <div class="row"><b>${esc(l.title)}</b><span class="${l.ok ? 'good' : 'bad'}">${l.ok ? 'fits' : 'doesn’t fit'}</span></div>
      <div class="meta">${l.report.tray_count} tray${l.report.tray_count === 1 ? '' : 's'} · ${l.report.layer_count} layer${l.report.layer_count === 1 ? '' : 's'} · ${mm(l.report.lid_clearance.mm)} mm under lid · <code>${esc(l.strategy)}</code></div>
    </button>`).join('');
  for (const b of $('variants').querySelectorAll('.variant')) {
    b.addEventListener('click', () => b.dataset.strategy !== B.chosen && command({ cmd: 'choose_layout', box: B.id, strategy: b.dataset.strategy }));
  }

  renderTrays();

  // Layer slider.
  // What lies on top of the insert (a board) is one more layer.
  // Imported pieces carry the guide layer they show from.
  const pieceLayers = (s.pieces || []).filter((p) => (p.box || state.boxes[0]?.id) === B.id).flatMap((p) => p.copies.map((c) => (c.at ? c.layer || 1 : 0)));
  const n = Math.max(L.layers.length + (L.on_top?.length ? 1 : 0), ...pieceLayers);
  const sl = $('layer');
  sl.max = Math.max(n, 1);
  sl.disabled = n <= 1;
  if (view.upTo === Infinity || view.upTo > n) { sl.value = sl.max; view.upTo = Infinity; $('layer-out').textContent = 'all'; }

  $('t-art').hidden = !!B.printed || !Object.keys(state.textures?.[B.id] || {}).length;
  $('t-lid').hidden = !B.printed;
  drawBox(B, L);
  fitCamera(B.size.inner, false);
  applyView();
  renderTabs();
  renderTrayTab();
  plates.render();
  renderStatus();
}

// ---------- tabs ----------
function renderTabs() {
  if (tab === 'tray' && !trayTab) tab = 'insert';
  $('tab-components').hidden = tab !== 'components';
  $('tab-insert').hidden = tab !== 'insert';
  $('tab-tray').hidden = tab !== 'tray';
  $('tab-plates').hidden = tab !== 'plates';
  plates?.show(tab === 'plates');
  const tt = trayTab && state?.layout?.trays.find((t) => t.id === trayTab);
  $('tab-btn-tray').hidden = !trayTab;
  $('tab-btn-tray').textContent = tt ? `Tray: ${tt.name}` : 'Tray';
  // One 3D view, shown in whichever of Insert and Tray is open.
  const viewer = document.querySelector('.viewer');
  const home = tab === 'tray' ? $('tab-tray') : $('tab-insert');
  if (tab !== 'plates' && viewer.parentElement !== home) { home.prepend(viewer); requestAnimationFrame(resize); }
  for (const b of document.querySelectorAll('.tab')) {
    b.classList.toggle('on', b.dataset.tab === tab);
    b.setAttribute('aria-selected', b.dataset.tab === tab);
  }
}
function showTab(t) {
  const was = tab;
  tab = t;
  if (t !== 'tray') measure?.set(false);
  setParam('tab', t === 'insert' ? null : t);
  renderTabs();
  if (t === 'insert') requestAnimationFrame(resize);
  // The Tray tab always shows its tray alone; leaving it for Insert goes back to the whole insert.
  if (t === 'tray' && state?.layout) { setFocus({ id: trayTab, mode: 'tray' }); renderTrayTab(); }
  if (t === 'insert' && was === 'tray' && focus?.mode === 'tray') setFocus(null);
}

function openTray(id) {
  trayTab = id;
  setParam('tray', id);
  showTab('tray');
  // From the top: the tray's view and its panel, not wherever the list was scrolled to.
  window.scrollTo({ top: 0 });
  for (const el of document.querySelectorAll('.panel, .layout')) el.scrollTop = 0;
}
for (const b of document.querySelectorAll('.tab')) b.addEventListener('click', () => showTab(b.dataset.tab));

// ---------- boxes ----------
function renderBoxTabs() {
  const bs = state.boxes || [];
  $('box-tabs').hidden = bs.length < 2;
  $('box-tabs').innerHTML = bs.map((b) => `<button class="toggle ${b.id === curBox()?.id ? 'on' : ''}" data-box="${esc(b.id)}">${esc(b.name)}</button>`).join('');
  for (const b of $('box-tabs').querySelectorAll('[data-box]')) b.addEventListener('click', () => pickBox(b.dataset.box));
}

function pickBox(id) {
  if (id === curBox()?.id) return;
  boxId = id;
  setParam('box', id);
  selected = null; fitted = false; glbVersion = null;
  if (focus?.mode === 'box' || focus?.mode === 'tray' || focus?.mode === 'tray-box') { focus = null; $('focus-bar').hidden = true; }
  if (tab === 'tray') showTab('insert');
  trayTab = null; setParam('tray', null);
  disposeGroup(modelGroup);
  render();
  loadBuild();
}

function renderBoxes() {
  const bs = state.boxes || [];
  const p = state.spec.printer, st = state.style;
  const prop = (k) => (st.proposed || []).includes(k) ? ' <span class="badge um">PROPOSED</span>' : '';
  const r = state.layout?.report;
  $('boxinfo').innerHTML = bs.map((b) => {
    const n = state.placement.filter((x) => x.box === b.id && x.status !== 'undecided').length;
    return `<div class="box-row ${b.id === curBox()?.id ? 'sel' : ''}">
      <div class="row"><b>${esc(b.name)}</b> <code class="muted small">${esc(b.id)}</code>
        <span class="spacer"></span><button class="toggle" data-show-box="${esc(b.id)}">Show</button><button class="toggle" data-edit-box="${esc(b.id)}">Edit</button></div>
      <div class="d">${b.printed
        ? (b.shell ? `To print, sized to fit: ${dims(b.shell.outer)} outside, ${dims(b.shell.inner)} inside · slide-on lid` : 'To print, sized to fit once it holds something · slide-on lid')
          + `${b.printed.max_inner ? ` · at most ${b.printed.max_inner.map((v) => v ? mm(v) : 'any').join(' × ')} mm inside` : ''}${b.size.top_stack ? ` · top stack ${mm(b.size.top_stack)} mm` : ''}`
        : `${dims(b.size.inner)} inside <span class="muted">(${esc(b.size.source)})</span> · telescoping lid${b.size.lid_depth ? `, ${mm(b.size.lid_depth)} mm room above the rim` : ''}${b.size.lid_lift ? `, lift up to ${mm(b.size.lid_lift)} mm` : ''} · top stack ${mm(b.size.top_stack)} mm`} · ${n} component${n === 1 ? '' : 's'}</div></div>`;
  }).join('') + `<button class="toggle" id="box-add" style="margin-top:8px">Add a box</button>
    <h2 style="margin-top:14px">Printer and style guide <code class="muted small">${esc(st.name)}</code></h2>
    <dl class="kv">
    <dt>Printer</dt><dd>${esc(p.name)}, bed ${p.bed.join(' × ')}</dd>
    <dt>Print ↔ part</dt><dd>${st.tolerance.print_to_component} mm per side, minimum</dd>
    <dt>Print ↔ print</dt><dd>${st.tolerance.print_to_print} mm</dd>
    <dt>Walls</dt><dd>${st.walls.normal} normal · ${st.walls.min} min · ${st.walls.absolute_min} tweak-only · ${st.walls.max} reinforced</dd>
    <dt>Floor</dt><dd>${st.floor.normal} normal${prop('floor.normal')} · ${st.floor.min} min${prop('floor.min')}</dd>
    <dt>Heavy cards</dt><dd>stacks ≥ ${st.heavy_cards_mm} mm get ${st.walls.max} mm walls${prop('heavy_cards_mm')}</dd>
    <dt>Overhangs</dt><dd>≥ ${st.max_overhang_deg}° from horizontal, no supports</dd>
    ${r ? `<dt>This layout</dt><dd title="${esc(r.wall.why)} · ${esc(r.divider.why)} · ${esc(r.floor.why)}">${esc(r.walls_level)}: ${mm(r.wall.mm)} walls, ${mm(r.divider.mm)} dividers, ${mm(r.floor.mm)} floor</dd>` : ''}
    <dt>SolidCraft</dt><dd>${esc(state.solidcraft)}</dd></dl>`;
  for (const b of $('boxinfo').querySelectorAll('[data-edit-box]')) b.addEventListener('click', () => editBox(b.dataset.editBox));
  for (const b of $('boxinfo').querySelectorAll('[data-show-box]')) b.addEventListener('click', () => pickBox(b.dataset.showBox));
  $('box-add').addEventListener('click', () => editBox(null));
}

// A box is a component: it is added and edited in the Components tab.
function editBox(id) {
  showTab('components');
  if (id) editComponent(id); else newComponent('box');
}

// ---------- where everything goes ----------
const STATUS = {
  placed: ['In the insert', 'good'], as_is: ['In the box as it is', ''], undecided: ['Not decided yet', 'warn'],
  unplaced: ['No place yet', 'bad'], no_box: ['No box yet', 'warn'],
};
let placementStale = false;

const REQ = { open: ['Waiting for the AI agent', 'warn'], working: ['The AI agent is on it', 'warn'], done: ['Done', 'good'], declined: ['Declined', 'bad'] };
function requestNote(r) {
  const [label, cls] = REQ[r.status] || ['', ''];
  const when = new Date(r.updated_ms).toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
  return `<div class="tray-note ${esc(r.status)}"><div><b>For the AI agent:</b> ${esc(r.text)}</div>
    <div class="small"><span class="${cls}">${label}</span> <span class="muted">· ${esc(when)}</span></div>
    ${r.reply ? `<div class="small"><b>AI agent:</b> ${esc(r.reply)}</div>` : ''}
    ${['done', 'declined'].includes(r.status) ? `<div class="form-actions"><span class="spacer"></span><button type="button" class="toggle" data-read="${esc(r.id)}" title="Clear the note and reply from the tray; they stay in its history">Mark read</button></div>` : ''}</div>`;
}
// A slot picker's menu closes on a click anywhere else.
document.addEventListener('click', (e) => {
  if (!e.target.closest?.('.slot-pick')) for (const m of document.querySelectorAll('.slot-menu')) m.hidden = true;
});
// "Mark read" on any answered request, wherever it's shown.
document.addEventListener('click', async (e) => {
  const b = e.target.closest?.('[data-read]');
  if (!b) return;
  const v = await command({ cmd: 'mark_read', id: b.dataset.read });
  if (v.ok) toast('Marked read: it\'s in the tray\'s history');
});
$('placement').addEventListener('focusout', () => setTimeout(() => { if (placementStale && !pressing) renderPlacement(); }, 0));

// The insert as a tree: layer → tray → the game components it holds → their pockets, then what
// has no tray (as-is, not decided, no place yet). Expanded rows stay open across live updates.
function renderPlacement() {
  if (!state?.spec) return;
  // Don't redraw under someone typing a tray's name or notes; catch up when they leave the field.
  const typing = document.activeElement?.closest?.('#placement') && /^(INPUT|TEXTAREA)$/.test(document.activeElement.tagName);
  if (typing) { placementStale = true; return; }
  placementStale = false;
  const rows = (state.placement || []).filter((x) => x.status !== 'box');
  const B = curBox();
  const here = rows.filter((x) => !B || !x.box || x.box === B.id);
  const multi = (state.boxes || []).length > 1;
  const placed = here.filter((x) => ['placed', 'as_is', 'in_box'].includes(x.status)).length;
  const pending = (B?.tweaks || []).filter((t) => t.request && ['open', 'working'].includes(t.request.status)).length;
  $('place-count').textContent = `(${placed} of ${here.length} have a place${multi ? ` in ${B?.name || ''}` : ''}${pending ? ` · ${pending} request${pending === 1 ? '' : 's'} for the AI agent` : ''})`;
  const open = new Set([...$('placement').querySelectorAll('details[open][data-key]')].map((d) => d.dataset.key));
  const comps = Object.fromEntries(state.spec.components.map((c) => [c.id, c]));
  const tweaks = Object.fromEntries((B?.tweaks || []).map((t) => [t.tray, t]));
  const L = state.layout;
  const btns = (id, canAlone, canBox) => `<span class="row-btns">
    <button class="toggle ${focus?.id === id && focus.mode === 'alone' ? 'on' : ''}" data-alone="${esc(id)}" ${canAlone ? '' : 'disabled'}>Alone</button>
    <button class="toggle ${focus?.id === id && focus.mode === 'box' ? 'on' : ''}" data-inbox="${esc(id)}" ${canBox ? '' : 'disabled'}>In box</button></span>`;
  const pocketLine = (p) => p.imported ? `<li>${esc(p.label)}${p.items > 1 ? ` <span class="muted">— ${p.items}</span>` : ''}</li>` : `<li><details data-key="p:${esc(p.id)}" ${open.has(`p:${p.id}`) ? 'open' : ''}><summary>${esc(p.label)} <span class="muted">— ${p.shape === 'round' ? `⌀${mm(p.size[0])}` : `${mm(p.size[0])} × ${mm(p.size[1])}`}, ${mm(p.depth)} deep${p.finger_hole ? ', finger hole' : ''}${p.open_ends ? ', open ends' : ''}${p.scoop ? ', scoop' : ''}</span></summary>
    <div class="why">length: ${esc(p.why.length)}</div><div class="why">width: ${esc(p.why.width)}</div><div class="why">depth: ${esc(p.why.depth)}</div></details></li>`;
  const trayNode = (t) => {
    const tw = tweaks[t.id];
    // Its game components, in pocket order; empty pockets (Spare) after them.
    const byComp = new Map();
    for (const p of t.pockets) {
      const k = p.items > 0 ? p.component : '';
      if (!byComp.has(k)) byComp.set(k, []);
      byComp.get(k).push(p);
    }
    const kids = [...byComp].filter(([k]) => k).map(([id, ps]) => {
      const c = comps[id];
      const items = ps.reduce((n, p) => n + p.items, 0);
      return `<div class="node comp-node"><details data-key="c:${esc(t.id)}:${esc(id)}" ${open.has(`c:${t.id}:${id}`) ? 'open' : ''}>
        <summary><span class="node-main"><b>${esc(c?.name || id)}</b> <span class="k">${esc(c?.kind || '')}</span>
          <span class="muted small">${ps.length} pocket${ps.length === 1 ? '' : 's'}${items > 1 ? ` · ${items}` : ''}</span></span>${btns(id, !!(c && mainDims(c)), true)}</summary>
        <ul class="pockets">${ps.map(pocketLine).join('')}</ul></details></div>`;
    });
    const spare = byComp.get('') || [];
    if (spare.length) kids.push(`<div class="node comp-node spare"><details data-key="s:${esc(t.id)}" ${open.has(`s:${t.id}`) ? 'open' : ''}><summary><span class="node-main muted">${spare.length} empty pocket${spare.length === 1 ? '' : 's'}</span></summary><ul class="pockets">${spare.map(pocketLine).join('')}</ul></details></div>`);
    const on = (mode) => focus?.mode === mode && focus.id === t.id;
    return `<div class="node tray-node ${selected === t.id ? 'sel' : ''}" data-tray="${esc(t.id)}">
      <div class="tray-head"><span class="swatch" style="background:${esc(t.color)}"></span><b>${esc(t.name)}</b>
        <span class="spacer"></span><span class="row-btns"><button class="toggle ${on('tray') ? 'on' : ''}" data-tray-focus="tray" data-id="${esc(t.id)}">Alone</button>
          <button class="toggle ${on('tray-box') ? 'on' : ''}" data-tray-focus="tray-box" data-id="${esc(t.id)}">In box</button>
          <button class="toggle" data-tray-open="${esc(t.id)}">Open</button></span></div>
      <div class="d tray-dims">${dims(t.size)} · ${t.imported ? `imported as it is${t.inside ? `, inside ${esc(t.inside)}` : ''}` : `${mm(t.wall)} wall`}${t.bed_fit ? '' : ' · <span class="bad">too big for the bed</span>'}</div>
      ${t.name !== t.auto_name ? `<div class="why">renamed from ${esc(t.auto_name)}</div>` : ''}
      ${t.request ? requestNote(t.request) : ''}
      ${configNotes(t, tw, comps, `cfg:${t.id}`, open.has(`cfg:${t.id}`))}
      ${selected === t.id ? trayEditForm(t) : ''}
      <div class="kids">${kids.join('')}</div></div>`;
  };
  const looseNode = (x, st) => `
      <div class="node comp-node loose"><div class="summary-row"><span class="node-main"><b>${esc(x.name)}</b> <span class="k">${esc(x.kind === 'as_is' ? 'as-is' : x.kind)}</span></span>${btns(x.id, !!mainDims(comps[x.id]), st === 'as_is' || st === 'in_box')}</div>
        <div class="d">${esc(x.detail)}</div></div>`;
  // Changes for the AI agent about the insert as a whole, and their history.
  const whole = (B?.tweaks || []).find((x) => x.tray === '*');
  let html = !B ? '' : `<div class="insert-notes">
    ${whole?.request ? requestNote(whole.request) : ''}
    ${trayEditForm({ id: '*', request: whole?.request }, 'the whole insert')}
    <details data-key="ins-hist" class="tray-history" ${open.has('ins-hist') ? 'open' : ''}><summary class="small"><b>History</b> <span class="muted">— notes and answers, changes asked for directly, every saved change</span></summary>
      <div id="insert-history" class="hist-list">${insertHist.box === B.id && insertHist.html ? insertHist.html : '<p class="muted small">Loading…</p>'}</div></details>
  </div>`;
  const shown = new Set();
  if (L) {
    // Boards and such lie on a layer; what goes on top of the insert is the layer above the trays'.
    const boardsOn = (n) => here.filter((x) => x.status === 'in_box' && (comps[x.id]?.place?.layer || 1) === n);
    const onTop = here.filter((x) => x.status === 'as_is');
    const placedTop = Math.max(0, ...here.filter((x) => x.status === 'in_box').map((x) => comps[x.id]?.place?.layer || 1));
    const n = Math.max(L.layers.length + (onTop.length ? 1 : 0), placedTop);
    for (let li = 0; li < n; li++) {
      const layer = L.layers[li];
      const ts = L.trays.filter((t) => t.layer === li);
      const boards = boardsOn(li + 1);
      const top = !layer && onTop.length && li === n - 1 ? onTop : [];
      for (const x of [...boards, ...top]) shown.add(x.id);
      const what = layer
        ? (ts.length ? `${mm(layer.height)} mm${L.layers.length > 1 ? `, from ${mm(layer.z)}` : ''} · ${ts.length} tray${ts.length === 1 ? '' : 's'}` : 'no trays')
        : top.length ? 'on top of the insert, as it is' : 'above the trays';
      html += `<div class="layer-head">Layer ${li + 1} <span class="muted small">· ${what}${boards.length ? ` · ${boards.length} board${boards.length === 1 ? '' : 's'} and such` : ''}</span></div>${ts.map(trayNode).join('')}${boards.map((x) => looseNode(x, 'in_box')).join('')}${top.map((x) => looseNode(x, 'as_is')).join('')}`;
    }
  }
  const loose = [
    ['in_box', 'Boards and other pieces lying in the box', ''], ['as_is', 'On top of the insert, as it is', ''], ['unplaced', 'No place yet', 'bad'], ['undecided', 'Not decided yet', 'warn'], ['no_box', 'No box yet', 'warn'],
  ];
  for (const [st, title, cls] of loose) {
    const xs = here.filter((x) => x.status === st && !shown.has(x.id));
    if (!xs.length) continue;
    html += `<div class="layer-head ${cls}">${title} <span class="muted small">· ${xs.length}</span></div>` + xs.map((x) => looseNode(x, st)).join('');
  }
  if (multi && rows.length > here.length) html += `<p class="muted small">${rows.length - here.length} more in other boxes.</p>`;
  $('placement').innerHTML = html || '<p class="muted">No components yet. Add them in the Components tab.</p>';
  const stop = (e) => { e.preventDefault(); e.stopPropagation(); };
  for (const b of $('placement').querySelectorAll('[data-alone]')) b.addEventListener('click', (e) => { stop(e); toggleFocus(b.dataset.alone, 'alone'); });
  for (const b of $('placement').querySelectorAll('[data-inbox]')) b.addEventListener('click', (e) => { stop(e); toggleFocus(b.dataset.inbox, 'box'); });
  for (const b of $('placement').querySelectorAll('[data-tray-open]')) b.addEventListener('click', (e) => { stop(e); openTray(b.dataset.trayOpen); });
  for (const b of $('placement').querySelectorAll('[data-tray-focus]')) {
    const { id, trayFocus: mode } = b.dataset;
    b.addEventListener('click', (e) => { stop(e); setFocus(focus?.mode === mode && focus.id === id ? null : { id, mode }); showView(); });
  }
  for (const el of $('placement').querySelectorAll('.tray-head')) el.addEventListener('click', () => select(el.parentElement.dataset.tray, false));
  wireTrayEdit($('placement'));
  const ih = $('placement').querySelector('[data-key="ins-hist"]');
  ih?.addEventListener('toggle', () => { if (ih.open) loadInsertHistory(); });
  if (ih?.open) loadInsertHistory();
}

function toggleFocus(id, mode) {
  setFocus(focus?.id === id && focus.mode === mode ? null : { id, mode });
  showView();
}

// On a phone or portrait iPad the view sits above the panel: bring it back into sight.
function showView() {
  if (focus && matchMedia('(max-width: 900px)').matches) $('view').scrollIntoView({ behavior: 'smooth', block: 'start' });
}

// Look at one component: alone (its measured shape, with its photo) or picked out in its box.
function setFocus(f) {
  focus = f;
  if (picked && !(f?.mode === 'tray' && f.id === picked.tray)) pickBin(null);
  measure?.clear();
  disposeGroup(soloGroup);
  const bar = $('focus-bar');
  if (!f) { bar.hidden = true; applyView(); renderPlacement(); renderTrays(); if (curBox()) fitCamera(curBox().size.inner, true); return; }
  if (f.mode === 'tray' || f.mode === 'tray-box') {
    const t = state.layout?.trays.find((x) => x.id === f.id);
    if (!t) { setFocus(null); return; }
    // The Tray tab names the tray and the view controls carry Contents, so it needs no bar.
    bar.hidden = tab === 'tray';
    bar.innerHTML = `<span><b>${esc(t.name)}</b></span>
      <span class="toggles"><button class="toggle" id="focus-open">Open in Tray tab</button>
      <button class="toggle" id="focus-close">Back to the whole insert</button></span>`;
    $('focus-close').addEventListener('click', () => setFocus(null));
    $('focus-open').addEventListener('click', () => openTray(t.id));
    applyView();
    renderPlacement();
    renderTrays();
    // In its box: close on the tray, with room for what's around it, which pulses.
    fitTray(t, f.mode === 'tray' ? 1 : 1.7);
    if (f.mode === 'tray-box') pulseAround(true);
    if (tab === 'components') showTab('insert');
    return;
  }
  const c = state.spec.components.find((x) => x.id === f.id);
  if (f.mode === 'box') {
    const box = state.placement.find((x) => x.id === f.id)?.box;
    if (box && box !== curBox()?.id) pickBox(box);
  }
  if (f.mode === 'alone') buildSolo(c);
  // From the Tray tab, a component alone stays in that tab's view, with a way back to the tray.
  const inTray = tab === 'tray' && f.mode === 'alone';
  bar.hidden = false;
  bar.innerHTML = `<span><b>${esc(c.name)}</b> · ${f.mode === 'alone' ? esc(describeMeasure(c) || 'no measurements') : 'in its box'}</span>
    <button class="toggle" id="focus-close">${inTray ? 'Back to the tray' : 'Back to the whole insert'}</button>`;
  $('focus-close').addEventListener('click', () => setFocus(inTray ? { id: trayTab, mode: 'tray' } : null));
  applyView();
  renderPlacement();
  if (f.mode === 'alone') fitSolo();
  else if (curBox()) fitCamera(curBox().size.inner, true);
  if (tab !== 'insert' && !inTray) showTab('insert');
}

function buildSolo(c) {
  const d = mainDims(c);
  if (!d) return;
  const [w, dd, h] = d;
  const top = state.textures?.[c.id]?.top;
  const round = c.kind === 'mini' || (c.kind === 'tokens' && c.shape === 'round') || c.measure?.shape === 'round';
  const side = new THREE.MeshStandardMaterial({ color: 0xa89878 });
  const topMat = new THREE.MeshStandardMaterial(top ? { map: texFor(top) } : { color: 0xd8cba8 });
  let mesh;
  if (c.kind === 'box' && c.printed) {
    // The printed box closed, as its chosen layout sized it.
    const sh = state.boxes.find((x) => x.id === c.id)?.shell;
    const [oL, oW, oH] = sh?.outer || [100, 80, 40];
    const body = new THREE.Mesh(new THREE.BoxGeometry(oL, oW, oH), new THREE.MeshStandardMaterial({ color: 0xb9c0ca }));
    body.position.set(0, 0, oH / 2);
    const span = Math.ceil(oL * 1.4 / 10) * 10;
    soloGroup.add(body, new THREE.GridHelper(span, span / 10, 0x9a978e, 0xc9c6bd).rotateX(Math.PI / 2));
    soloGroup.userData.size = [oL, oW, oH];
    return;
  }
  if (c.kind === 'box') {
    // Closed: the lid sits on the base, its skirt reaching down as far as it telescopes, and
    // riding as high as the chosen layout's lid lift.
    const g = boxShapes(c, true);
    const b = state.boxes.find((x) => x.id === c.id);
    const lift = b?.layouts.find((l) => l.strategy === b.chosen)?.report.lid_lift.mm || 0;
    const base = new THREE.Mesh(g.base, g.baseMats);
    base.position.set(0, 0, (g.oH - BOARD) / 2);
    const lid = new THREE.Mesh(g.lid, g.lidMats);
    lid.position.set(0, 0, g.oH + lift - g.skirt / 2);
    const span = Math.ceil(g.size[0] * 1.4 / 10) * 10;
    soloGroup.add(base, lid, new THREE.GridHelper(span, span / 10, 0x9a978e, 0xc9c6bd).rotateX(Math.PI / 2));
    soloGroup.userData.size = g.size;
    return;
  } else if (round) {
    mesh = new THREE.Mesh(new THREE.CylinderGeometry(Math.max(w, dd) / 2, Math.max(w, dd) / 2, h, 48).rotateX(Math.PI / 2), [side, topMat, side]);
  } else {
    mesh = new THREE.Mesh(new THREE.BoxGeometry(w, dd, h), [side, side, side, side, topMat, side]);
  }
  mesh.position.set(0, 0, h / 2);
  const edges = new THREE.LineSegments(new THREE.EdgesGeometry(mesh.geometry), new THREE.LineBasicMaterial({ color: 0x55524a }));
  edges.position.copy(mesh.position);
  // A 10 mm grid underneath for scale.
  const span = Math.ceil(Math.max(w, dd) * 1.6 / 10) * 10;
  const grid = new THREE.GridHelper(span, span / 10, 0x9a978e, 0xc9c6bd);
  grid.rotateX(Math.PI / 2);
  soloGroup.add(mesh, edges, grid);
  const front = state.textures?.[c.id]?.front;
  if (front) {
    const t = texFor(front);
    const card = new THREE.Mesh(new THREE.PlaneGeometry(Math.max(w, dd), h).rotateX(Math.PI / 2), new THREE.MeshStandardMaterial({ map: t, side: THREE.DoubleSide, transparent: true }));
    card.position.set(0, -dd / 2 - 0.2, h / 2);
    soloGroup.add(card);
  }
  soloGroup.userData.size = [w, dd, h];
}

// Frame one tray where it sits in the box.
// From its meshes where it's drawn (an imported tray's pos is in the frame it sits in).
function fitTray(t, room = 1) {
  const b = new THREE.Box3();
  for (const m of modelGroup.children) if (m.userData.kind === 'tray' && m.userData.tray === t.id) b.expandByObject(m);
  const [x, y, z] = b.isEmpty() ? t.pos : b.min.toArray();
  const [L, W, H] = b.isEmpty() ? t.size : b.getSize(new THREE.Vector3()).toArray();
  const c = new THREE.Vector3(x + L / 2, y + W / 2, z + H / 3);
  const r = Math.max(Math.hypot(L, W, H), 40) * room;
  camera.position.set(c.x - r * 0.55, c.y - r * 1.05, c.z + r * 0.95);
  controls.target.copy(c);
  camera.near = r / 100; camera.far = r * 30; camera.updateProjectionMatrix();
}

function fitSolo() {
  const [w, d, h] = soloGroup.userData.size || [50, 50, 10];
  const r = Math.max(Math.hypot(w, d, h), 30);
  camera.position.set(-r * 0.6, -r * 1.1, r * 0.9);
  controls.target.set(0, 0, h / 3);
  camera.near = r / 100; camera.far = r * 40; camera.updateProjectionMatrix();
}

// ---------- the Tray tab ----------
// One tray: its name and requests for the AI agent, its settings (tweaks), what it holds and why it's
// that size. The 3D view beside it shows the tray alone.
let trayTabKey = null;
let trayTabStale = false;
// A press in progress: a redraw waits for its click to land.
let pressing = false;
document.addEventListener('pointerdown', () => { pressing = true; }, true);
document.addEventListener('pointerup', () => setTimeout(() => {
  pressing = false;
  if (placementStale && !document.activeElement?.closest?.('#placement')) renderPlacement();
  if (trayTabStale && !document.activeElement?.closest?.('#tray-panel')) { trayTabStale = false; trayTabKey = null; renderTrayTab(); }
}, 0), true);
let trayHistoryOpen = false;
let trayConfigOpen = false;
function renderTrayTab() {
  if (tab !== 'tray' || !state?.layout) return;
  const L = state.layout, B = curBox();
  const t = L.trays.find((x) => x.id === trayTab);
  const tw = (B?.tweaks || []).find((x) => x.tray === trayTab) || {};
  // Like the tree: don't redraw under typing.
  const typing = document.activeElement?.closest?.('#tray-panel') && /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement.tagName);
  const key = JSON.stringify([t, tw, L.trays.map((x) => [x.id, x.name]), state.plates?.pieces.find((p) => p.part === trayTab), state.plates?.names, state.plates?.filaments]);
  if (typing) { if (key !== trayTabKey) trayTabStale = true; return; }
  if (key === trayTabKey) return;
  trayTabKey = key;
  const picker = `<div class="row tray-picker">
      <button class="toggle" data-step="-1" aria-label="Previous tray">‹</button>
      <select id="tray-pick" aria-label="Tray">${L.trays.map((x) => `<option value="${esc(x.id)}" ${x.id === trayTab ? 'selected' : ''}>Layer ${x.layer + 1} · ${esc(x.name)}</option>`).join('')}</select>
      <button class="toggle" data-step="1" aria-label="Next tray">›</button></div>`;
  if (!t) {
    $('tray-panel').innerHTML = `<section class="card"><h2>Tray</h2>${picker}<p class="warn small">This tray isn't in the ${esc(B?.name || '')} layout any more; pick another.</p></section>`;
    wireTrayPicker();
    return;
  }
  const comps = Object.fromEntries(state.spec.components.map((c) => [c.id, c]));
  // Its print piece: the plate it's on and its filament slots.
  const piece = state.plates?.pieces.find((p) => p.part === t.id && p.box === B.id);
  const slots = state.plates?.filaments || [];
  // A filament slot picker that shows each slot's colour (a native select can't, everywhere).
  const slotText = (i) => `Slot ${i + 1} · ${slots[i].colour_name || slots[i].color} ${slots[i].preset.replace(/ @.*/, '')}`;
  const slotItem = (i) => `<span class="swatch" style="background:${esc(slots[i]?.color || '#9b9ea0')}"></span><span>${esc(slots[i] ? slotText(i) : `Slot ${i + 1}`)}</span>`;
  const slotPick = (name, v) => `<div class="slot-pick"><input type="hidden" name="${esc(name)}" value="${v}">
    <button type="button" class="slot-btn" aria-haspopup="listbox">${slotItem(v - 1)}<span class="muted">▾</span></button>
    <div class="slot-menu" role="listbox" hidden>${slots.map((_, i) => `<button type="button" role="option" data-slot="${i + 1}" aria-selected="${v === i + 1}">${slotItem(i)}</button>`).join('')}</div></div>`;
  // Colours the other trays use, to match: one square per colour, the trays in its tooltip.
  const used = new Map();
  for (const x of L.trays) if (x.id !== t.id && x.color) used.set(x.color.toLowerCase(), [...(used.get(x.color.toLowerCase()) || []), x.name]);
  const usedSwatches = used.size ? `<div class="used-colours" aria-label="Colours other trays use">${[...used].map(([c, names]) => `<button type="button" class="swatch" data-use-colour="${esc(c)}" style="background:${esc(c)}" title="${esc(`${c}: ${names.join(', ')}`)}" aria-label="${esc(`${c}, used by ${names.join(', ')}`)}"></button>`).join('')}</div>` : '';
  $('tray-panel').innerHTML = `
    <section class="card">
      ${picker}
      <h2 class="tray-title"><span class="swatch" style="background:${esc(t.color)}"></span>${esc(t.name)}</h2>
      ${t.name !== t.auto_name ? `<p class="muted small">renamed from ${esc(t.auto_name)}</p>` : ''}
      <dl class="kv">
        <dt>Size</dt><dd>${dims(t.size)}</dd>
        <dt>Where</dt><dd>layer ${t.layer + 1} of ${L.layers.length} in ${esc(B.name)}, at ${mm(t.pos[0])}, ${mm(t.pos[1])} mm</dd>
        ${t.imported ? `<dt>From</dt><dd>${esc(t.source || 'an imported project')}, as it is</dd>` : `<dt>Walls · floor</dt><dd>${mm(t.wall)} · ${mm(t.floor)} mm</dd>`}
        <dt>Bed</dt><dd class="${t.bed_fit ? 'good' : 'bad'}">${t.bed_fit ? 'fits the X1C bed' : 'too big for the bed'}</dd>
        ${piece ? `<dt>Plate</dt><dd><a href="#" data-goto-plate="${esc(piece.id)}">${esc(plateName(piece.plate))}</a></dd>` : ''}
      </dl>
      ${configNotes(t, tw, comps, 'cfg-tab', trayConfigOpen)}
    </section>
    <section class="card">
      <h2>Changes for AI agent</h2>
      ${t.request ? requestNote(t.request) : ''}
      ${trayEditForm(t)}
      <details id="tray-history-wrap" class="tray-history" ${trayHistoryOpen ? 'open' : ''}><summary class="small"><b>History</b> <span class="muted">— notes and answers, changes asked for directly, saved changes</span></summary>
        <div id="tray-history" class="hist-list"><p class="muted small">Loading…</p></div></details>
    </section>
    ${t.imported ? (t.design ? `<section class="card">
      <h2>Edit in SolidCraft</h2>
      <p class="muted small">Made in SolidCraft: opens its design there in a new tab. Each save updates the piece here, each body a part.</p>
      <div class="form-actions"><span class="spacer"></span><button class="primary" type="button" id="sc-piece">Edit in SolidCraft</button></div>
      <p class="small" id="sc-status" data-key="piece:${esc(t.piece)}" aria-live="polite">${esc(editorStatus(game, `piece:${t.piece}`))}</p>
    </section>` : '') : `<section class="card">
      <h2>Edit in SolidCraft</h2>
      <p class="muted small">Opens this tray in SolidCraft in a new tab, with its full history to change by hand. What you change is compared with the generated tray here: send it to the AI agent as a note (it then lasts through every later change), or keep it as the tray (it prints as you made it).</p>
      ${keptNote(t)}
      <div class="form-actions"><span class="spacer"></span><button class="primary" type="button" id="sc-open">Edit in SolidCraft</button></div>
      <p class="small" id="sc-status" data-key="${esc(t.id)}" aria-live="polite">${esc(editorStatus(game, t.id))}</p>
      <div id="sc-diff"></div>
    </section>`}
    <section class="card">
      <h2>Settings</h2>
      <form id="tray-settings" class="form">
        <label>Name <input name="name" value="${esc(t.name !== t.auto_name ? t.name : '')}" placeholder="${esc(t.auto_name)}"></label>
        <div class="two">
          <div><label>Colour in the app <input name="color" type="color" value="${esc(tw.color || t.color)}"></label>${usedSwatches}</div>
          <label class="check"><input name="color_auto" type="checkbox" ${tw.color ? '' : 'checked'}> Packer’s colour</label>
        </div>
        ${piece ? `<div class="two">
          <label>Filament ${slotPick('filament', piece.filament)}</label>
          ${piece.parts.map((x) => `<label>${x.name === 'Label' ? 'Label filament' : `${esc(x.name)} filament`} ${slotPick(`part:${x.name}`, x.filament)}</label>`).join('')}
        </div>` : ''}
        <div class="form-actions"><span class="spacer"></span><button class="primary" type="submit">Save settings</button></div>
      </form>
    </section>
    <section class="card">
      <h2>What it holds <span class="muted small">· ${t.imported ? `${t.pockets.length} component${t.pockets.length === 1 ? '' : 's'}` : `${t.pockets.length} bin${t.pockets.length === 1 ? '' : 's'}`}</span></h2>
      ${binList(t, L, comps)}
    </section>`;
  wireTrayPicker();
  loadTrayHistory(t.id);
  if (!t.imported) loadEditDiff(t.id);
  $('sc-open')?.addEventListener('click', () => openEditor(game, B.id, t.id, t.name));
  $('sc-piece')?.addEventListener('click', () => openPiece(game, t.piece, t.name));
  $('sc-drop')?.addEventListener('click', async () => {
    const v = await command({ cmd: 'drop_edit', box: B.id, tray: t.id });
    if (v.ok) toast(v.message);
  });
  wireTrayEdit($('tray-panel'));
  for (const b of $('tray-panel').querySelectorAll('[data-hold-alone]')) b.addEventListener('click', () => setFocus({ id: b.dataset.holdAlone, mode: 'alone' }));
  for (const b of $('tray-panel').querySelectorAll('[data-pick-bin]')) b.addEventListener('click', () => {
    const p = t.pockets.find((x) => x.id === b.dataset.pickBin);
    pickBin(picked?.pocket === p.id ? null : { tray: t.id, pocket: p.id });
    for (const x of $('tray-panel').querySelectorAll('[data-pick-bin]')) { const on = picked?.pocket === x.dataset.pickBin; x.classList.toggle('on', on); x.setAttribute('aria-pressed', on); x.closest('.bin')?.classList.toggle('picked', on); }
  });
  $('tray-history-wrap').addEventListener('toggle', (e) => { trayHistoryOpen = e.target.open; });
  $('tray-panel').querySelector('[data-key="cfg-tab"]')?.addEventListener('toggle', (e) => { trayConfigOpen = e.target.open; });
  $('tray-panel').querySelector('[data-goto-plate]')?.addEventListener('click', (e) => { e.preventDefault(); plates.focus(e.currentTarget.dataset.gotoPlate); showTab('plates'); });
  for (const pk of $('tray-panel').querySelectorAll('.slot-pick')) {
    const menu = pk.querySelector('.slot-menu');
    pk.querySelector('.slot-btn').addEventListener('click', () => {
      const open = menu.hidden;
      for (const m of $('tray-panel').querySelectorAll('.slot-menu')) m.hidden = true;
      menu.hidden = !open;
    });
    for (const o of menu.querySelectorAll('[data-slot]')) o.addEventListener('click', () => {
      pk.querySelector('input').value = o.dataset.slot;
      pk.querySelector('.slot-btn').innerHTML = `${slotItem(o.dataset.slot - 1)}<span class="muted">▾</span>`;
      for (const x of menu.querySelectorAll('[data-slot]')) x.setAttribute('aria-selected', x === o);
      menu.hidden = true;
    });
  }
  for (const b of $('tray-panel').querySelectorAll('[data-use-colour]')) b.addEventListener('click', (e) => {
    e.preventDefault();
    const f = $('tray-settings');
    f.color.value = b.dataset.useColour;
    f.color_auto.checked = false;
  });
  $('tray-settings').addEventListener('submit', async (e) => {
    e.preventDefault();
    const f = e.target;
    const body = { cmd: 'tweak_tray', box: B.id, tray: t.id, name: f.name.value, unset: [] };
    if (f.color_auto.checked) body.unset.push('color'); else body.color = f.color.value;
    let v = await command(body);
    if (v.ok && piece) {
      const parts = {};
      for (const x of piece.parts) { const n = +f[`part:${x.name}`].value; if (n !== x.filament) parts[x.name] = n; }
      const fil = +f.filament.value;
      if (fil !== piece.filament || Object.keys(parts).length) v = await command({ cmd: 'set_plate', piece: piece.id, ...(fil !== piece.filament ? { filament: fil } : {}), parts });
    }
    if (v.ok) { toast('Saved'); document.activeElement?.blur(); }
  });
}

// A tray kept as a SolidCraft edit: since when, what happens when the tray changes, and what
// the last build made of it (re-applied, or not applied).
function keptNote(t) {
  const k = t.kept_edit;
  if (!k) return '';
  const said = (curBox()?.build?.warnings || []).filter((w) => w.subject === t.id && w.code.startsWith('edit_'));
  return `<div class="kept-edit">
    <p class="small"><b>Prints as your SolidCraft edit</b>, kept ${esc(new Date(k.kept_ms).toLocaleString())}. If the generated tray changes, ${k.added === 1 ? 'the feature you added goes' : `the ${k.added} features you added go`} on the new one${k.changed ? `; your ${k.changed} change${k.changed === 1 ? '' : 's'} to its own features don't` : ''}.</p>
    ${said.map((w) => `<p class="small ${w.code === 'edit_not_applied' ? 'warn' : 'muted'}">${esc(w.message)}</p>`).join('')}
    <div class="form-actions"><span class="spacer"></span><button type="button" id="sc-drop">Print as generated again</button></div>
  </div>`;
}

// The last SolidCraft edit of a tray, compared with the generated tray.
initSolidcraft({
  command,
  onStatus: (g, key, text) => { const el = $('sc-status'); if (g === game && el?.dataset.key === key) el.textContent = text; },
  onPiece: () => {},
});
async function loadEditDiff(id) {
  const v = await command({ cmd: 'edit_diff', tray: id });
  const el = $('sc-diff');
  if (!el || trayTab !== id) return;
  const d = v.ok ? v.data : null;
  if (!d) { el.innerHTML = ''; return; }
  const pic = (f, alt) => `<figure><img src="/api/games/${esc(game)}/media/edits/${esc(f)}?v=${d.saved_ms}" alt="${esc(alt)}" loading="lazy"><figcaption class="muted small">${esc(alt)}</figcaption></figure>`;
  el.innerHTML = `<h3 class="small">Last edit, ${esc(new Date(d.saved_ms).toLocaleString())}</h3>
    <ul class="small">${d.summary.map((l) => `<li>${esc(l)}</li>`).join('')}</ul>
    ${d.pictures ? `<div class="two">${pic(`${id}.generated.png`, 'Generated')}${pic(`${id}.png`, 'Edited in SolidCraft')}</div>` : ''}
    <div class="form-actions"><span class="spacer"></span><button type="button" id="sc-note">Put in a note for the AI agent</button>
      <button type="button" id="sc-keep" title="Print this tray as you made it in SolidCraft, instead of as generated">${state?.layout?.trays.find((x) => x.id === id)?.kept_edit ? 'Keep this edit instead' : 'Keep as the tray'}</button></div>`;
  $('sc-keep').addEventListener('click', async () => {
    const v = await command({ cmd: 'keep_edit', box: curBox()?.id, tray: id });
    if (v.ok) toast(v.message);
  });
  $('sc-note').addEventListener('click', () => {
    const ta = $('tray-panel').querySelector(`[data-tray-edit="${CSS.escape(id)}"] textarea`);
    if (!ta) return;
    // Added to what's being written; the last request's own text (as drawn) is replaced.
    const own = ta.value.trim() && ta.value !== ta.defaultValue;
    ta.value = own ? `${ta.value.trim()}\n\n${noteFrom(d)}` : noteFrom(d);
    ta.dispatchEvent(new Event('input', { bubbles: true }));
    ta.scrollIntoView({ block: 'center' });
    ta.focus();
    toast('In the note: add what you want, then Save');
  });
}

// "3 · Tokens and rules", or "Plate 3".
function plateName(n) {
  const name = state.plates?.names?.[n - 1];
  return name ? `Plate ${n} · ${name}` : `Plate ${n}`;
}

// What's been set for this tray beyond its name and colour: the AI agent sets these when asked.
function designChoices(tw, comps) {
  const says = {
    max_depth: (v) => `deepest pocket ${v} mm`, walls: (v) => `${v} mm walls`, finger_holes: (v) => `finger holes ${v ? 'on' : 'off'}`,
    open_ends: (v) => (v ? 'open-ended card holders' : 'closed ends'), per_pocket: (v) => (v ? 'one tray per pocket' : ''), holders: (v) => (v ? 'a frame of lift-out holders' : ''),
    grips: (v) => (v ? 'pull lips on the holders' : ''), half_circle: (v) => `grab holes ${v === 'middle' ? 'in the middle' : `just past ${comps[v]?.name || v}`}`,
    scoops: (v) => `scoops ${v ? 'on' : 'off'}`, label: (v) => `label “${v}” on top`, pocket_labels: (v) => `pocket labels ${v ? 'on' : 'off'}`,
    label_mode: (v) => `labels ${v === 'cut' ? 'cut' : 'inlaid'}`, order: (v) => `pockets in the order ${v.map((c) => comps[c]?.name || c).join(', ')}`,
    pockets: (v) => Object.entries(v).map(([c, p]) => `${comps[c]?.name || c}: ${[p.scoop === false ? 'flat floor' : p.scoop ? 'scoop' : '', p.notch ? `${p.notch.width} mm notch, ${p.notch.keep} mm of wall kept` : '', p.fill_spare ? 'takes the spare room' : ''].filter(Boolean).join(', ')}`).join('; '),
  };
  return Object.entries(says).filter(([k]) => tw[k] !== undefined && tw[k] !== null).map(([k, f]) => f(tw[k])).filter(Boolean);
}
// A tray's config notes, collapsed: what's been set for it (the AI agent sets these when asked)
// and why the packer made it the size it is.
function configNotes(t, tw, comps, key, isOpen) {
  const set = tw ? designChoices(tw, comps) : [];
  const why = t.imported ? [] : [['height', t.why.height], ['footprint', t.why.footprint], ['walls', t.why.walls]];
  if (!set.length && !why.length) return '';
  return `<details class="config-notes" data-key="${esc(key)}" ${isOpen ? 'open' : ''}><summary class="small muted">Config notes</summary>
    ${set.length ? `<div class="small"><b>Set for this tray</b> <span class="muted">— ask the AI agent to change these</span></div><ul class="pockets">${set.map((x) => `<li class="small">${esc(x)}</li>`).join('')}</ul>` : ''}
    ${why.length ? `<div class="small"><b>Why this size</b> <span class="muted">— the packer's working</span></div>${why.map(([k, v]) => `<div class="why">${k}: ${esc(v)}</div>`).join('')}` : ''}
  </details>`;
}

// What a tray holds, bin by bin (each pocket, in the tray's order): its size and why, what goes
// in it (with Alone), and its floor label; the tray's top label first. A bin can be picked out
// in the view. Labels are placed by the packer, so this is where they're seen, not set.
let picked = null; // { tray, pocket }
function binList(t, L, comps) {
  const missed = (L.warnings || []).filter((w) => w.code === 'label_no_room' && w.subject === t.id);
  const labelLine = (l) => `<li class="small"><details><summary>Label “${esc(l.text)}” <span class="muted">— ${mm(l.height)} mm, ${l.mode === 'inlay' ? 'inlaid' : 'cut'}${l.angle ? ', across' : ''}</span></summary><div class="why">${esc(l.why)}</div></details></li>`;
  const missLine = (w) => `<li class="small warn">Label left off: ${esc(w.message.replace(/^[^:]+: /, ''))}</li>`;
  const out = [];
  const top = (t.labels || []).filter((l) => l.on === 'top');
  const topMiss = missed.filter((w) => /the tray's top|a frame's top/.test(w.message));
  if (top.length || topMiss.length) out.push(`<div class="bin"><div class="row"><b>Tray top</b></div><ul class="pockets">${top.map(labelLine).join('')}${topMiss.map(missLine).join('')}</ul></div>`);
  t.pockets.forEach((p, n) => {
    const c = p.items > 0 ? comps[p.component] : null;
    const size = p.shape === 'round' ? `⌀${mm(p.size[0])}` : `${mm(p.size[0])} × ${mm(p.size[1])}`;
    const extras = [p.finger_hole && 'finger hole', p.open_ends && 'open ends', p.floor_window && 'floor window', p.scoop ? 'scoop' : 'flat floor',
      p.notch && `${mm(p.notch.width)} mm notch in its long walls, ${mm(p.notch.keep)} mm above the floor`].filter(Boolean).join(', ');
    const on = picked?.tray === t.id && picked.pocket === p.id;
    const labels = (t.labels || []).filter((l) => l.on === p.id);
    const miss = missed.filter((w) => w.message.includes(`the floor of ${p.id}:`) || w.message.includes(`'${p.label}'`) && !labels.length && w.message.includes('floor'));
    const head = p.imported ? `<b>${esc(p.label)}</b>` : `<details><summary><b>Bin ${n + 1}</b> <span class="muted small">— ${size}, ${mm(p.depth)} deep${extras ? `, ${extras}` : ''}</span></summary>
        <div class="why">length: ${esc(p.why.length)}</div><div class="why">width: ${esc(p.why.width)}</div><div class="why">depth: ${esc(p.why.depth)}</div></details>`;
    out.push(`<div class="bin ${on ? 'picked' : ''}"><div class="row">${head}<span class="spacer"></span>
        ${p.imported ? '' : `<button class="toggle ${on ? 'on' : ''}" data-pick-bin="${esc(p.id)}" aria-pressed="${on}">Highlight</button>`}</div>
      <ul class="pockets">
        ${c ? `<li class="row"><span>${esc(c.name)} <span class="k">${esc(c.kind)}</span>${p.items > 1 ? ` <span class="muted small">× ${p.items}</span>` : ''}${p.label !== c.name ? ` <span class="muted small">(${esc(p.label)})</span>` : ''}</span><span class="spacer"></span><button class="toggle" data-hold-alone="${esc(c.id)}">Alone</button></li>`
          : `<li class="small muted">${p.items > 0 ? esc(p.label) : 'Empty: spare room'}</li>`}
        ${labels.map(labelLine).join('')}${miss.map(missLine).join('')}
      </ul></div>`);
  });
  return out.join('');
}

// Light up one bin of the tray shown: a see-through block filling the pocket, and its outline.
function pickBin(want) {
  picked = want;
  disposeGroup(pickGroup);
  const t = want && state?.layout?.trays.find((x) => x.id === want.tray);
  const p = t?.pockets.find((x) => x.id === want.pocket);
  if (!p) { picked = null; return; }
  const top = t.pos[2] + t.size[2];
  const g = p.shape === 'round'
    ? new THREE.CylinderGeometry(p.size[0] / 2, p.size[0] / 2, p.depth, 48).rotateX(Math.PI / 2)
    : new THREE.BoxGeometry(p.size[0], p.size[1], p.depth);
  const [cx, cy] = p.shape === 'round' ? [p.pos[0] + p.size[0] / 2, p.pos[1] + p.size[0] / 2] : [p.pos[0] + p.size[0] / 2, p.pos[1] + p.size[1] / 2];
  g.translate(t.pos[0] + cx, t.pos[1] + cy, top - p.depth / 2 + 0.05);
  const fill = new THREE.Mesh(g, new THREE.MeshBasicMaterial({ color: 0xf2a23a, transparent: true, opacity: 0.45, depthWrite: false }));
  const edge = new THREE.LineSegments(new THREE.EdgesGeometry(g), new THREE.LineBasicMaterial({ color: 0xf2a23a }));
  pickGroup.add(fill, edge);
}

function wireTrayPicker() {
  $('tray-pick').addEventListener('change', (e) => openTray(e.target.value));
  for (const b of $('tray-panel').querySelectorAll('[data-step]')) {
    b.addEventListener('click', () => {
      const ids = state.layout.trays.map((x) => x.id);
      const i = ids.indexOf(trayTab);
      openTray(ids[(i + +b.dataset.step + ids.length) % ids.length]);
    });
  }
}

// A tray's story, newest first, from tray_history.
// A tray's (or the whole insert's) story as HTML, newest first.
function historyHtml(list) {
  const when = (ms) => new Date(ms).toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
  return list.map((h) => {
    if (h.kind === 'change') return `<div class="hist"><div class="row"><span class="badge">Change</span><span class="muted small">${esc(when(h.time_ms))}</span></div><div class="small">${esc(h.text)}</div></div>`;
    const r = h.request, [label, cls] = REQ[r.status] || ['', ''];
    const tag = h.kind === 'ask' ? 'Asked directly' : 'Note';
    const state = r.read_ms ? `read ${when(r.read_ms)}` : `<span class="${cls}">${label}</span>`;
    return `<div class="hist named"><div class="row"><span class="badge ver">${tag}</span><span class="muted small">${esc(when(r.created_ms))} · ${state}</span></div>
      <div class="small"><b>${h.kind === 'ask' ? 'Asked' : 'For the AI agent'}:</b> ${esc(r.text)}</div>
      ${r.reply ? `<div class="small"><b>AI agent:</b> ${esc(r.reply)}</div>` : ''}</div>`;
  }).join('') || '<p class="muted small">Nothing yet.</p>';
}
async function loadTrayHistory(id) {
  const v = await command({ cmd: 'tray_history', box: curBox()?.id, tray: id });
  const el = $('tray-history');
  if (!el || trayTab !== id) return;
  if (!v.ok) { el.innerHTML = `<p class="muted small">${esc(v.reason?.message || 'No history')}</p>`; return; }
  el.innerHTML = historyHtml(v.data.history);
}
// The whole insert's: its notes and answers, and every saved change. Kept, so a redraw of the
// tree doesn't flash "Loading…".
let insertHist = { box: null, html: '' };
async function loadInsertHistory() {
  const box = curBox()?.id;
  const v = await command({ cmd: 'tray_history', box, tray: '*' });
  if (!v.ok) return;
  insertHist = { box, html: historyHtml(v.data.history) };
  if ($('insert-history')) $('insert-history').innerHTML = insertHist.html;
}

// Changes for the AI agent on a tray. Its name is in the Tray tab's Settings.
function trayEditForm(t, about = 'this tray') {
  return `<form class="tray-edit form" data-tray-edit="${esc(t.id)}">
    <label>Changes for AI agent <span class="muted small">— what to change about ${about}; an agent working the request queue picks it up</span>
      <textarea name="notes" rows="3" placeholder="e.g. make the coin pockets shallower, room for 20 more cards">${esc(drafts.get(draftKey(t.id)) ?? t.request?.text ?? '')}</textarea></label>
    ${drafts.has(draftKey(t.id)) ? '<p class="warn small">Not sent yet: press Save.</p>' : ''}
    <p class="muted small">Saving changed text sends it as a new request; empty removes it.</p>
    <div class="form-actions"><span class="spacer"></span><button class="primary" type="submit">Save</button></div>
  </form>`;
}

// Note text not yet saved, by tray ('*': the whole insert): a redraw (a live update, the server
// coming back) puts it back rather than losing it. Gone once it's saved.
const drafts = new Map();
const draftKey = (id) => `${curBox()?.id || ''}/${id}`;
document.addEventListener('input', (e) => {
  const f = e.target.closest?.('[data-tray-edit]');
  if (f && e.target.name === 'notes') drafts.set(draftKey(f.dataset.trayEdit), e.target.value);
});
function wireTrayEdit(root) {
  for (const f of root.querySelectorAll('[data-tray-edit]')) {
    f.addEventListener('submit', async (e) => {
      e.preventDefault();
      const btn = f.querySelector('[type=submit]');
      btn.disabled = true;
      let v;
      try { v = await command({ cmd: 'tweak_tray', box: curBox()?.id, tray: f.dataset.trayEdit, notes: f.notes.value }); }
      catch { v = { ok: false }; toast('Not sent: the server can’t be reached. Your text is kept; try again.'); }
      btn.disabled = false;
      if (v.ok) { drafts.delete(draftKey(f.dataset.trayEdit)); toast(f.notes.value.trim() ? 'Sent to the AI agent' : 'Removed'); document.activeElement?.blur(); }
    });
  }
}
// Catch up on what changed while typing, once the field is left; never mid-click, or the click
// (Save) would land on a button the redraw just replaced.
$('tray-panel').addEventListener('focusout', () => setTimeout(() => { if (trayTabStale && !pressing) { trayTabStale = false; trayTabKey = null; renderTrayTab(); } }, 0));

// ---------- save history ----------
// games/ is its own git repository; the server commits each game ~30 s after its changes settle.
function renderSaveStatus() {
  const h = state?.history;
  const el = $('save-status');
  $('history-btn').hidden = !h?.enabled;
  if (!h?.enabled) { el.textContent = ''; return; }
  if (h.push_error) { el.innerHTML = '<span class="warn">Saved here, not backed up</span>'; el.title = h.push_error; return; }
  // Saving is automatic (the server commits each change, the agent's too), so only a problem shows.
  el.title = '';
  el.textContent = '';
}

let liveOn = false;
const liveLine = () => liveOn
  ? '<span class="live on">●</span> Live: changes made elsewhere (the AI agent, another browser) show here at once'
  : '<span class="live off">●</span> Not connected to the server: reconnecting; changes made elsewhere show after a reload';
async function openHistory() {
  openDialog('Status', `
    <p class="muted small" id="hist-status"></p>
    <p class="muted small" id="hist-live">${liveLine()}</p>
    <fieldset><legend>Save a version</legend>
      <label>Name <input name="name" required placeholder="e.g. before the drawer idea"></label>
      <label>Note <span class="muted small">— optional</span> <input name="note"></label>
      <div class="form-actions"><span class="spacer"></span><button class="primary" type="submit">Save version</button></div>
    </fieldset>
    <div id="hist-list" class="hist-list"><p class="muted small">Loading…</p></div>`, async (f) => {
    const v = await command({ cmd: 'save_version', name: f.name.value, note: f.note.value || null });
    if (v.ok) { toast(`Saved version “${f.name.value}”`); f.name.value = ''; f.note.value = ''; loadHistory(); }
    return false;
  });
  loadHistory();
}

async function loadHistory() {
  const v = await command({ cmd: 'history', limit: 200 });
  if (!v.ok || $('dialog').hidden) return;
  const st = v.data.status;
  $('hist-status').innerHTML = `<b class="${st.pending_changes ? 'warn' : 'good'}">${st.pending_changes ? `Saving… ${st.pending_changes} change${st.pending_changes === 1 ? '' : 's'} waiting` : 'All changes saved'}</b><br>
    Every change is kept, yours and the AI agent's: the server saves ${esc(state.spec.game.name)} ${st.quiet_s} s after the changes stop.
    ${st.push_error ? `<br><span class="warn">Not backed up to GitHub: ${esc(st.push_error)}</span>` : ''}`;
  const when = (ms) => new Date(ms).toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
  $('hist-list').innerHTML = v.data.history.map((r, i) => `
    <div class="hist ${r.versions.length ? 'named' : ''}">
      <div class="row">${r.versions.map((x) => `<span class="badge ver" title="${esc(x.note)}">${esc(x.name)}</span>`).join('')}
        <span class="muted small">${esc(when(r.time_ms))}</span><span class="spacer"></span>
        ${i === 0 ? '<span class="muted small">current</span>' : `<button type="button" class="toggle" data-restore="${esc(r.commit)}" data-label="${esc(r.versions[0]?.name || when(r.time_ms))}">Restore</button>`}</div>
      <div class="small">${esc(r.subject)}</div>
      ${r.changes.length > 1 ? `<details class="small"><summary>${r.changes.length} changes</summary><ul>${r.changes.map((c) => `<li>${esc(c)}</li>`).join('')}</ul></details>` : ''}
      ${r.versions.map((x) => x.note ? `<div class="muted small">${esc(x.note)}</div>` : '').join('')}
    </div>`).join('') || '<p class="muted small">Nothing saved yet.</p>';
  for (const b of $('hist-list').querySelectorAll('[data-restore]')) {
    b.addEventListener('click', async () => {
      if (!confirm(`Put ${state.spec.game.name} back as it was at “${b.dataset.label}”? What's there now stays in the history.`)) return;
      const r = await command({ cmd: 'restore_version', rev: b.dataset.restore });
      if (r.ok) { toast('Restored'); loadHistory(); }
    });
  }
}
$('history-btn').addEventListener('click', openHistory);

// ---------- dialog ----------
let dialogSubmit = null;
function openDialog(title, html, onSubmit) {
  $('dialog-title').textContent = title;
  $('dialog-form').innerHTML = html;
  dialogSubmit = onSubmit;
  $('dialog').hidden = false;
  $('dialog-form').querySelector('input')?.focus();
}
function closeDialog() { $('dialog').hidden = true; dialogSubmit = null; }
$('dialog-close').addEventListener('click', closeDialog);
$('dialog-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const btn = $('dialog-form').querySelector('[type=submit]');
  btn.disabled = true;
  const done = dialogSubmit ? await dialogSubmit($('dialog-form').elements) : true;
  btn.disabled = false;
  if (done) closeDialog();
});

const newProject = () => openDialog('New project', `
  <label>Game <input name="name" required placeholder="The game's name"></label>
  <p class="muted small">Starts on the Components tab. Add the box when you've measured it.</p>
  <div class="form-actions"><span class="spacer"></span><button class="primary" type="submit">Create project</button></div>`, async (f) => {
  const name = f.name.value.trim();
  let slug = slugify(name) || 'game';
  const have = (await (await fetch('/api/games')).json()).games;
  for (let i = 2; have.includes(slug); i++) slug = `${slugify(name)}-${i}`;
  const r = await fetch('/api/command', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ cmd: 'create_game', game: slug, name }) });
  const v = await r.json();
  if (!v.ok) { toast(v.reason?.message || 'could not create'); return false; }
  boxId = null; setParam('box', null);
  showTab('components');
  await switchGame(slug, true);
  return true;
});

function renderTrays() {
  renderPlacement();
}

function renderStatus() {
  const s = $('build-status');
  if (!state) return;
  renderSaveStatus();
  const build = curBox()?.build;
  if (state.build_error && (!build || state.build_error.version > build.version)) {
    s.innerHTML = `<span class="bad">Build failed</span>`;
    s.title = state.build_error.reason.message;
    $('view-note').hidden = false;
    $('view-note').textContent = `SolidCraft build failed: ${state.build_error.reason.message}`;
    return;
  }
  $('view-note').hidden = true;
  if (state.building) { s.textContent = 'Building…'; s.title = ''; return; }
  if (build) {
    // The agent's status has the corner now; the build time stays on hover.
    s.textContent = '';
    s.title = `Built in ${(build.total_ms / 1000).toFixed(1)} s: ${build.commands} SolidCraft commands, ${build.solidcraft_ms} ms in solidcraft-cli ${state.solidcraft}`;
  } else {
    s.textContent = curBox() ? 'Nothing to build' : 'No box yet';
    s.title = '';
  }
}

// ---------- data ----------
let loading = null;
async function refresh() {
  if (!game) return;
  if (loading) { loading.again = true; return; }
  loading = { again: false };
  try {
    const r = await fetch(`/api/games/${encodeURIComponent(game)}/state`);
    if (r.ok) {
      state = await r.json();
      render();
      dressGhosts();
      await loadBuild();
    }
  } catch (e) {
    console.error(e);
  } finally {
    const again = loading.again;
    loading = null;
    if (again) refresh();
  }
}

// The GLB for the box on show, when it's newer than what's loaded.
async function loadBuild() {
  const build = curBox()?.build;
  if (!build) { if (glbVersion !== null) { glbVersion = null; disposeGroup(modelGroup); } return; }
  const key = `${curBox().id}@${build.version}`;
  if (key === glbVersion) return;
  glbVersion = key;
  await loadGlb(build.glb);
}

// The project's name is the switcher: a menu of the projects, and New project.
let games = { games: [], names: {} };
async function loadGames() {
  games = await (await fetch('/api/games')).json();
  if (!game) game = games.current || games.games[0];
  renderProjectMenu();
}
function renderProjectMenu() {
  $('project-menu').innerHTML = games.games.map((s) => `<button type="button" role="menuitemradio" aria-checked="${s === game}" data-project="${esc(s)}">${esc(games.names?.[s] || s)}</button>`).join('')
    + '<hr><button type="button" role="menuitem" id="project-new">New project…</button>';
  for (const b of $('project-menu').querySelectorAll('[data-project]')) b.addEventListener('click', () => { closeProjectMenu(); switchGame(b.dataset.project, true); });
  $('project-new').addEventListener('click', () => { closeProjectMenu(); newProject(); });
}
function closeProjectMenu() { $('project-menu').hidden = true; $('game-name').setAttribute('aria-expanded', 'false'); }
$('game-name').addEventListener('click', async () => {
  if (!$('project-menu').hidden) { closeProjectMenu(); return; }
  await loadGames();
  $('project-menu').hidden = false;
  $('game-name').setAttribute('aria-expanded', 'true');
  $('project-menu').querySelector('[aria-checked="true"]')?.focus();
});
document.addEventListener('click', (e) => { if (!e.target.closest?.('#project-menu, #game-name')) closeProjectMenu(); });
document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && !$('project-menu').hidden) { closeProjectMenu(); $('game-name').focus(); } });

async function switchGame(slug, fromPicker) {
  if (slug === game) return;
  game = slug; glbVersion = null; fitted = false; selected = null; state = null; focus = null;
  if (fromPicker) { boxId = null; setParam('box', null); }
  disposeGroup(modelGroup); disposeGroup(soloGroup); $('focus-bar').hidden = true;
  if (fromPicker) { follow = false; const u = new URL(location); u.searchParams.set('game', slug); history.replaceState(null, '', u); }
  await loadGames();
  await refresh();
}

function connect() {
  const es = new EventSource('/api/events');
  // The header shows the live link only when it's down; the History box always says.
  const live = (on) => { liveOn = on; $('live').hidden = on; if (!$('dialog').hidden && $('hist-live')) $('hist-live').innerHTML = liveLine(); };
  es.onopen = () => live(true);
  es.onerror = () => live(false);
  es.onmessage = (m) => {
    const ev = JSON.parse(m.data);
    if (ev.type === 'hello') { loadGames().then(refresh); return; }
    if (ev.type === 'current') { if (follow) switchGame(ev.game, false); else loadGames(); return; }
    if (ev.game !== game) return;
    if (ev.type === 'agent' && ev.status) { agent = ev.status; renderAgent(); }
    if (ev.type === 'history' && !$('dialog').hidden && $('hist-list')) loadHistory();
    if (ev.type === 'history' && !$('dialog').hidden && $('hist-list')) loadHistory();
    if (ev.type === 'edit' && ev.game === game && tab === 'tray' && ev.tray === trayTab) loadEditDiff(ev.tray);
    if (ev.type === 'export') { showExport(ev.dir, ev.files); toast(`Exported ${ev.files.length} files`); return; }
    if (ev.type === 'spec' || ev.type === 'building') { if (state) { state.building = true; renderStatus(); } }
    refresh();
  };
}


// ---------- textured placeholders ----------
// Each ghost from the GLB whose component has a face texture is replaced by a three.js model at
// exactly the ghost's size and place: art on top (cards, tokens, bins), a photo standing on its
// base for minis. Ghosts without photos stay plain.
const texLoader = new THREE.TextureLoader();
const texCache = new Map();
let dressedKey = null;

function texFor(url) {
  if (!texCache.has(url)) {
    const t = texLoader.load(url, () => {});
    t.colorSpace = THREE.SRGBColorSpace;
    t.anisotropy = 4;
    texCache.set(url, t);
  }
  return texCache.get(url);
}

// Paper edges for a card stack: fine stripes.
const edgeTex = (() => {
  const c = document.createElement('canvas');
  c.width = 8; c.height = 64;
  const g = c.getContext('2d');
  for (let y = 0; y < 64; y++) { g.fillStyle = y % 2 ? '#f3f1ea' : '#d9d5ca'; g.fillRect(0, y, 8, 1); }
  const t = new THREE.CanvasTexture(c);
  t.wrapS = t.wrapT = THREE.RepeatWrapping;
  t.colorSpace = THREE.SRGBColorSpace;
  return t;
})();

// A bin of measured loose pieces (tokens): each piece is drawn, as many as the count, rather
// than one block for the bin. Irregular pieces (dials, stands) keep the block.
function loosePieces(c) {
  const m = c.measure;
  if (c.kind !== 'bin' || !m?.piece || !(m.count > 1) || m.shape === 'irregular') return null;
  return { size: m.piece, count: m.count, shape: m.shape || (Math.abs(m.piece[0] - m.piece[1]) < 0.5 ? 'square' : 'rect') };
}

// The pieces lying loose in the bin: a grid per layer with a little give in each cell, each
// turned at random (the same every time), layers stacked to the count.
function scatter(p, box, top) {
  const [a, b, t] = p.size;
  const round = p.shape === 'round' || p.shape === 'hex';
  const geo = round
    ? new THREE.CylinderGeometry(a / 2, a / 2, t, p.shape === 'hex' ? 6 : 36).rotateX(Math.PI / 2)
    : new THREE.BoxGeometry(a, b, t);
  const side = new THREE.MeshStandardMaterial({ color: 0x3b4250 });
  const face = new THREE.MeshStandardMaterial(top ? { map: top } : { color: 0xc8b98f });
  const mats = round ? [side, face, side] : [side, side, side, side, face, side];
  const mesh = new THREE.InstancedMesh(geo, mats, p.count);
  const size = box.getSize(new THREE.Vector3());
  const reach = round ? a : Math.hypot(a, b) * 0.85;
  const cols = Math.max(1, Math.floor(size.x / reach)), rows = Math.max(1, Math.floor(size.y / reach));
  const cell = [size.x / cols, size.y / rows];
  const rnd = (i) => { const x = Math.sin(i * 12.9898 + 78.233) * 43758.5453; return x - Math.floor(x); };
  const o = new THREE.Object3D();
  for (let i = 0; i < p.count; i++) {
    const layer = Math.floor(i / (cols * rows)), k = i % (cols * rows);
    // Upper layers sit a little offset into the gaps of the one below; every piece stays inside.
    const shift = layer % 2 ? 0.25 : -0.25;
    const slack = [Math.max(cell[0] - reach, 0), Math.max(cell[1] - reach, 0)];
    const at = (lo, n, g, sl, r, hi) => Math.min(Math.max(lo + (g + 0.5 + (n > 1 ? shift : 0)) * (hi - lo) / n + (r - 0.5) * sl, lo + reach / 2), hi - reach / 2);
    o.position.set(
      at(box.min.x, cols, k % cols, slack[0], rnd(i), box.max.x),
      at(box.min.y, rows, Math.floor(k / cols), slack[1], rnd(i + 99), box.max.y),
      box.min.z + t / 2 + layer * t + 0.05,
    );
    o.rotation.set(0, 0, rnd(i + 7) * Math.PI * 2);
    o.updateMatrix();
    mesh.setMatrixAt(i, o.matrix);
  }
  mesh.computeBoundingSphere();
  return mesh;
}

function dressGhosts() {
  if (!state?.spec || !modelGroup.children.length) return;
  const key = JSON.stringify(state.textures || {}) + glbVersion;
  if (key === dressedKey) return;
  dressedKey = key;
  for (const m of [...modelGroup.children]) {
    if (m.userData.dressed) { m.removeFromParent(); m.geometry?.dispose(); }
    else if (m.userData.replaced) m.userData.replaced = false;
  }
  const comps = Object.fromEntries(state.spec.components.map((c) => [c.id, c]));
  for (const m of [...modelGroup.children]) {
    const u = m.userData;
    const faces = state.textures?.[u.component];
    const comp = comps[u.component];
    const pieces = comp && loosePieces(comp);
    if (u.kind !== 'ghost' || !comp || !(faces || pieces)) continue;
    const box = new THREE.Box3().setFromObject(m);
    const size = box.getSize(new THREE.Vector3());
    const c = box.getCenter(new THREE.Vector3());
    const made = [];
    const top = faces?.top && texFor(faces.top);
    if (pieces) {
      made.push(scatter(pieces, box, top));
      u.replaced = true;
      for (const x of made) {
        x.userData = { ...x.userData, kind: 'ghost', dressed: true, layer: u.layer, tray: u.tray, component: u.component, baseZ: x.position.z };
        modelGroup.add(x);
      }
      continue;
    }
    const round = comp.kind === 'mini' || (comp.kind === 'tokens' && comp.shape === 'round');
    if (comp.kind === 'mini') {
      const baseH = Math.min(2, size.z);
      const base = new THREE.Mesh(new THREE.CylinderGeometry(size.x / 2, size.x / 2, baseH, 40).rotateX(Math.PI / 2),
        [new THREE.MeshStandardMaterial({ color: 0x3a3d42 }), new THREE.MeshStandardMaterial(top ? { map: top } : { color: 0x3a3d42 }), new THREE.MeshStandardMaterial({ color: 0x3a3d42 })]);
      base.position.set(c.x, c.y, box.min.z + baseH / 2);
      made.push(base);
      if (faces.front) {
        const t = texFor(faces.front);
        const h = size.z - baseH;
        const fit = () => {
          const a = t.image ? t.image.width / t.image.height : 0.7;
          card.geometry.dispose();
          card.geometry = new THREE.PlaneGeometry(h * a, h).rotateX(Math.PI / 2);
        };
        const card = new THREE.Mesh(new THREE.PlaneGeometry(h * 0.7, h).rotateX(Math.PI / 2),
          new THREE.MeshStandardMaterial({ map: t, side: THREE.DoubleSide }));
        card.position.set(c.x, c.y, box.min.z + baseH + h / 2);
        card.userData.billboard = true;
        if (t.image) fit(); else pendingFits.push(fit);
        made.push(card);
      }
    } else if (round) {
      const side = new THREE.MeshStandardMaterial({ color: 0x8a7a5c });
      const mesh = new THREE.Mesh(new THREE.CylinderGeometry(size.x / 2, size.x / 2, size.z, 48).rotateX(Math.PI / 2),
        [side, new THREE.MeshStandardMaterial(top ? { map: top } : { color: 0xc8b98f }), side]);
      mesh.position.copy(c);
      made.push(mesh);
    } else {
      // Box faces: +x, −x, +y, −y, +z (top), −z.
      const edge = comp.kind === 'cards'
        ? () => { const t = edgeTex.clone(); t.repeat.set(1, Math.max(1, (size.z / 0.3) / 64)); t.needsUpdate = true; return new THREE.MeshStandardMaterial({ map: t }); }
        : () => new THREE.MeshStandardMaterial({ color: comp.kind === 'bin' ? 0xd0cfc9 : 0x8a7a5c });
      let topMat = new THREE.MeshStandardMaterial({ color: 0xc8b98f });
      if (top) {
        const t = top.clone();
        // The texture reads in the orientation it was marked; turn it if the pocket runs the other way.
        const orient = () => {
          const landscapeTex = t.image && t.image.width >= t.image.height;
          const landscapeBox = size.x >= size.y;
          t.center.set(0.5, 0.5);
          t.rotation = landscapeTex === landscapeBox ? 0 : Math.PI / 2;
          t.needsUpdate = true;
        };
        if (t.image) orient(); else pendingFits.push(orient);
        topMat = new THREE.MeshStandardMaterial({ map: t });
      }
      const mesh = new THREE.Mesh(new THREE.BoxGeometry(size.x, size.y, size.z), [edge(), edge(), edge(), edge(), topMat, edge()]);
      mesh.position.copy(c);
      made.push(mesh);
    }
    u.replaced = true;
    for (const x of made) {
      x.userData = { ...x.userData, kind: 'ghost', dressed: true, layer: u.layer, tray: u.tray, component: u.component, baseZ: x.position.z };
      modelGroup.add(x);
    }
  }
  applyView();
}
// Sizing and orientation that need a texture's image wait for it here.
const pendingFits = [];
texLoader.manager.onLoad = () => { for (const f of pendingFits.splice(0)) f(); };

// ---------- photo sheet ----------
let sheetFor = null;
let cropPhoto = null;
let cropFace = 'top';
let corners = null;

function openSheet(id) {
  sheetFor = id;
  cropPhoto = null;
  $('sheet').hidden = false;
  renderSheet();
}
$('sheet-close').addEventListener('click', () => { sheetFor = null; $('sheet').hidden = true; });

function facesFor(c) { return c.kind === 'box' ? ['lid', 'front', 'side'] : c.kind === 'mini' ? ['front', 'top'] : ['top']; }


function renderSheet() {
  const c = state?.spec?.components.find((x) => x.id === sheetFor);
  if (!c) { $('sheet').hidden = true; sheetFor = null; return; }
  $('sheet-title').textContent = `Photos and faces · ${c.name}`;
  let d = '';
  if (c.kind === 'cards') d = `Card ${c.size.join(' × ')} mm`;
  if (c.kind === 'tokens') d = `Token ${c.size.join(' × ')} mm`;
  if (c.kind === 'bin') d = `Bin ${c.size.join(' × ')} mm`;
  if (c.kind === 'mini') d = `Mini ⌀${c.base} × ${c.height} mm: mark the front (a photo from the side) and optionally the top`;
  if (c.kind === 'box') d = 'Box: mark the lid’s top, the front (a long side of the lid) and the side (a short side of the lid); the Box art toggle in the Insert tab shows them';
  $('sheet-dims').textContent = `${d}. The face is straightened to the measured size; it only dresses the viewer, never the print.`;
  const faceOf = Object.fromEntries(Object.entries(c.faces || {}).map(([k, f]) => [f.photo, k]));
  $('sheet-photos').innerHTML = (c.photos || []).map((p) => `
    <button class="thumb ${cropPhoto === p.file ? 'sel' : ''}" data-file="${esc(p.file)}">
      <img src="/api/games/${encodeURIComponent(game)}/media/photos/${encodeURIComponent(p.file)}" alt="">
      ${faceOf[p.file] ? `<span class="tag">${esc(faceOf[p.file])}</span>` : ''}</button>`).join('')
    + Object.entries(state.textures?.[c.id] || {}).map(([k, url]) => `
    <div class="thumb" title="${esc(k)} texture"><img src="${esc(url)}" alt=""><span class="tag">${esc(k)} texture</span></div>`).join('')
    || '<p class="muted small">No photos yet.</p>';
  for (const b of $('sheet-photos').querySelectorAll('[data-file]')) b.addEventListener('click', () => startCrop(c, b.dataset.file));
  $('crop').hidden = !cropPhoto;
}

$('photo-input').addEventListener('change', async (e) => {
  const file = e.target.files[0];
  e.target.value = '';
  if (!file || !sheetFor) return;
  toast('Uploading photo…');
  // Shrink on the device: a camera photo is several MB, 2560 px is plenty for a texture.
  const bmp = await createImageBitmap(file);
  const k = Math.min(1, 2560 / Math.max(bmp.width, bmp.height));
  const cv = document.createElement('canvas');
  cv.width = Math.round(bmp.width * k); cv.height = Math.round(bmp.height * k);
  cv.getContext('2d').drawImage(bmp, 0, 0, cv.width, cv.height);
  const v = await command({ cmd: 'add_photo', component: sheetFor, data: cv.toDataURL('image/jpeg', 0.88) });
  if (v.ok) {
    toast('Photo added: mark its face');
    await refresh();
    const c = state.spec.components.find((x) => x.id === sheetFor);
    startCrop(c, v.data.photo);
  }
});

function startCrop(c, file) {
  cropPhoto = file;
  const used = Object.entries(c.faces || {}).find(([, f]) => f.photo === file);
  cropFace = used ? used[0] : facesFor(c).find((f) => !c.faces?.[f]) || facesFor(c)[0];
  corners = used ? used[1].corners.map((p) => [...p]) : [[0.15, 0.15], [0.85, 0.15], [0.85, 0.85], [0.15, 0.85]];
  $('crop-faces').innerHTML = facesFor(c).map((f) => `<button class="toggle ${f === cropFace ? 'on' : ''}" data-face="${f}">${f}</button>`).join('');
  for (const b of $('crop-faces').querySelectorAll('[data-face]')) b.addEventListener('click', () => {
    cropFace = b.dataset.face;
    for (const x of $('crop-faces').children) x.classList.toggle('on', x === b);
    $('crop-face-name').textContent = cropFace;
  });
  $('crop-face-name').textContent = cropFace;
  const img = $('crop-img');
  img.onload = drawCrop;
  img.src = `/api/games/${encodeURIComponent(game)}/media/photos/${encodeURIComponent(file)}`;
  renderSheet();
  $('crop').scrollIntoView({ block: 'nearest', behavior: 'smooth' });
}

function drawCrop() {
  const img = $('crop-img');
  const w = img.clientWidth, h = img.clientHeight;
  const svg = $('crop-svg');
  svg.setAttribute('viewBox', `0 0 ${w} ${h}`);
  const pts = corners.map(([x, y]) => [x * w, y * h]);
  svg.innerHTML = `<polygon points="${pts.map((p) => p.join(',')).join(' ')}"></polygon>`
    + pts.map(([x, y], i) => `<circle data-i="${i}" cx="${x}" cy="${y}" r="16"></circle><text x="${x - 4}" y="${y + 5}">${i + 1}</text>`).join('');
}
new ResizeObserver(() => { if (cropPhoto) drawCrop(); }).observe($('crop-stage'));

let dragging = null;
$('crop-svg').addEventListener('pointerdown', (e) => {
  const i = e.target.dataset?.i;
  if (i === undefined) return;
  dragging = +i;
  $('crop-svg').setPointerCapture(e.pointerId);
  e.preventDefault();
});
$('crop-svg').addEventListener('pointermove', (e) => {
  if (dragging === null) return;
  const r = $('crop-img').getBoundingClientRect();
  corners[dragging] = [Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)), Math.min(1, Math.max(0, (e.clientY - r.top) / r.height))];
  drawCrop();
});
$('crop-svg').addEventListener('pointerup', () => { dragging = null; });
$('crop-svg').addEventListener('pointercancel', () => { dragging = null; });

$('crop-save').addEventListener('click', async () => {
  const v = await command({ cmd: 'set_face', component: sheetFor, face: cropFace, photo: cropPhoto, corners });
  if (v.ok) { toast(`Saved the ${cropFace} face`); await refresh(); }
});
$('crop-remove').addEventListener('click', async () => {
  if (!confirm('Delete this photo (and any face cropped from it)?')) return;
  const v = await command({ cmd: 'remove_photo', component: sheetFor, photo: cropPhoto });
  if (v.ok) { cropPhoto = null; await refresh(); }
});

// ---------- AI agent ----------
// Who works the trays' change requests: an agent polling from outside (a Claude session with
// bid / bid mcp), or the app's own, with a Claude API key set in Settings.
let agent = null;
const EFFORT_SAYS = { low: 'Low: quick, for small changes', medium: 'Medium', high: 'High (default): careful', xhigh: 'Extra high', max: 'Max: slowest, most thorough' };
function renderAgent() {
  const el = $('agent-status');
  if (!agent) { el.hidden = true; return; }
  el.hidden = false;
  const i = agent.inside, o = agent.outside;
  const model = agent.models.find((m) => m.id === i.model)?.name || i.model;
  const ago = (ms) => { const s = Math.round((Date.now() - ms) / 1000); return s < 60 ? `${s} s ago` : `${Math.round(s / 60)} min ago`; };
  const lines = [];
  // In the app: its key, model and thinking level, and what it's doing.
  if (i.key) {
    lines.push(`In the app: ${model}, thinking ${i.effort}${i.error ? ' (stopped)' : i.enabled ? '' : ' (paused)'}`);
    if (i.error) lines.push(`  ${i.error}`);
    if (i.busy) lines.push(`  Working on ${i.busy.tray_name} (${i.busy.game})`);
    else if (i.last) lines.push(`  Last: ${i.last.tray_name}, ${i.last.outcome}, ${i.last.input_tokens + i.last.output_tokens} tokens, ${ago(i.last.ended_ms || i.last.started_ms)}`);
  } else lines.push('In the app: no Claude API key (Settings)');
  // Outside: each session polling the queue, as bid described it.
  for (const c of o.clients || []) {
    const w = c.client;
    const via = [w.via, w.mcp_client && `from ${w.mcp_client}`, w.entrypoint && w.claude_session && `(Claude Code, ${w.entrypoint})`].filter(Boolean).join(' ');
    lines.push(`Outside: ${via}${w.host ? ` on ${w.host}` : ''}`);
    if (w.cwd) lines.push(`  in ${w.cwd}`);
    if (w.remote_session) lines.push(`  claude.ai/code/${w.remote_session}`);
    else if (w.claude_session) lines.push(`  session ${w.claude_session}`);
    lines.push(`  ${c.waiting ? 'waiting for requests now' : `last polled ${ago(c.seen_ms)}`}`);
  }
  if (!o.clients?.length) lines.push('Outside: nothing polling the change requests');
  const cls = i.key && i.error ? 'bad' : i.busy ? 'busy' : (i.active || o.watching) ? 'good' : 'none';
  el.className = `agent-chip ${cls}`;
  el.innerHTML = '<span class="dot"></span>AI agent';
  el.title = lines.join('\n');
  el.setAttribute('aria-label', `AI agent: ${lines[0]}`);
}
async function loadAgent() {
  const r = await fetch('/api/command', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ cmd: 'agent_status' }) });
  if (r.ok) { agent = (await r.json()).data; renderAgent(); }
}
setInterval(loadAgent, 30000);

function openSettings() {
  const i = agent?.inside || {};
  const models = (agent?.models || []).map((m) => `<option value="${esc(m.id)}" ${m.id === i.model ? 'selected' : ''}>${esc(m.name)}</option>`).join('');
  const efforts = (agent?.efforts || []).map((e) => `<option value="${e}" ${e === i.effort ? 'selected' : ''}>${esc(EFFORT_SAYS[e] || e)}</option>`).join('');
  openDialog('Settings', `
    <fieldset><legend>AI agent in the app</legend>
      <p class="muted small">With a Claude API key, the app works the trays’ change requests itself. Without one, an agent can still work them from outside (a Claude session running <code>bid mcp</code>).</p>
      ${i.key ? `<p class="small">API key set (${esc(i.key_hint)}) <button type="button" class="toggle" id="agent-remove-key">Remove key</button></p>
        <label>Replace key <input name="api_key" type="password" autocomplete="off" placeholder="sk-ant-…"></label>` :
        `<label>Claude API key <input name="api_key" type="password" autocomplete="off" placeholder="sk-ant-…"></label>`}
      ${i.key ? `<div class="two">
        <label>Model <select name="model">${models}</select></label>
        <label>Thinking level <select name="effort">${efforts}</select></label></div>
        <label class="check"><input name="enabled" type="checkbox" ${i.enabled ? 'checked' : ''}> Work new change requests</label>` : ''}
      ${i.error ? `<p class="bad small">${esc(i.error)}</p>` : ''}
      <p class="muted small">The key is checked with the Claude API, kept on this server in ${esc(agent?.settings_file || 'a private file')} (readable only by its user), and never sent back to a browser. Its use is billed to that key’s account.</p>
    </fieldset>
    <div class="form-actions"><span class="spacer"></span><button class="primary" type="submit">Save</button></div>`, async (f) => {
    const body = { cmd: 'set_agent' };
    if (f.api_key.value.trim()) body.api_key = f.api_key.value.trim();
    if (f.model) { body.model = f.model.value; body.effort = f.effort.value; body.enabled = f.enabled.checked; }
    const v = await command(body);
    if (!v.ok) return false;
    agent = v.data; renderAgent();
    toast(body.api_key && !f.model ? 'Key set: pick a model and thinking level' : 'Saved');
    if (body.api_key && !f.model) { openSettings(); return false; }
    return true;
  });
  $('agent-remove-key')?.addEventListener('click', async () => {
    if (!confirm('Remove the Claude API key? The app’s agent stops; requests wait for an agent outside.')) return;
    const v = await command({ cmd: 'set_agent', remove_key: true });
    if (v.ok) { agent = v.data; renderAgent(); openSettings(); }
  });
}
$('settings-btn').addEventListener('click', openSettings);
$('agent-status').addEventListener('click', openSettings);

// ---------- collapsible panel sections ----------
// Every panel card folds up by clicking its heading. Folds last this visit (by tab and heading,
// put back whenever a panel redraws); a reload starts everything open but the Fit report.
const folded = new Set();
try { localStorage.removeItem('bid.folded'); } catch {}
// Opened by hand where a card starts folded (data-fold="closed"); data-fold="open" holds it open.
const unfolded = new Set();
const isFolded = (card, key) => card.dataset.fold === 'open' ? false : card.dataset.fold === 'closed' ? !unfolded.has(key) : folded.has(key);
const foldKey = (card) => `${card.closest('main')?.id || ''}:${card.querySelector(':scope > h2')?.firstChild?.textContent.trim() || ''}`;
function applyFolds() {
  for (const card of document.querySelectorAll('.card')) {
    const h = card.querySelector(':scope > h2');
    if (!h) continue;
    const on = isFolded(card, foldKey(card));
    if (card.classList.contains('folded') !== on) card.classList.toggle('folded', on);
    if (h.getAttribute('aria-expanded') !== String(!on)) { h.setAttribute('aria-expanded', String(!on)); h.setAttribute('role', 'button'); h.tabIndex = 0; }
  }
}
function toggleFold(h) {
  const card = h.parentElement, key = foldKey(card);
  if (card.dataset.fold === 'open') return;
  const was = isFolded(card, key);
  if (card.dataset.fold === 'closed') { if (was) unfolded.add(key); else unfolded.delete(key); }
  else if (was) folded.delete(key); else folded.add(key);
  applyFolds();
}
document.addEventListener('click', (e) => {
  const h = e.target.closest?.('.card > h2');
  if (h && !e.target.closest('button, a, input, select, label')) toggleFold(h);
});
document.addEventListener('keydown', (e) => {
  if ((e.key === 'Enter' || e.key === ' ') && e.target.matches?.('.card > h2')) { e.preventDefault(); toggleFold(e.target); }
});
new MutationObserver(applyFolds).observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-fold'] });
applyFolds();

// ---------- start ----------
plates = initPlates({
  $, esc, mm, command, toast, getState: () => state, game: () => game,
  onExport: () => exportAndDownload(true),
  openPiece: (piece, name) => openPiece(game, piece, name),
});
initComponents({
  command, toast, openSheet, showTab,
  state: () => state, game: () => game, refresh,
  view: (id, mode) => setFocus({ id, mode }),
});
renderTabs();
await loadGames();
await refresh();
connect();
loadAgent();
