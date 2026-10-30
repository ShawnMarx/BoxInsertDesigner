// The Plates tab: every piece to print on the printer's plates, as the server's plate packer laid
// them out (`state.plates`), and as the Bambu Studio project the export writes. Tap a piece to
// turn it, turn it over, move it to another plate, or choose its filament (and its parts'); those
// go to the server as `set_plate`, and the packer places everything else around them. Plates take
// names, and the filament slots are set here too.
//
// Its own canvas and renderer: the plates are a different scene from the insert. Meshes come from
// `print.glb` (every piece as it prints, part by part), moved onto the plate by `print_to_plate`.

import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { LineSegments2 } from 'three/addons/lines/LineSegments2.js';
import { LineSegmentsGeometry } from 'three/addons/lines/LineSegmentsGeometry.js';
import { LineMaterial } from 'three/addons/lines/LineMaterial.js';

// Plates side by side in the view, this many bed widths apart, in rows of ⌈√n⌉.
const SPACING = 1.12;

export function initPlates(ctx) {
  // ctx: { $, esc, mm, command, toast, getState, game, onExport }
  const { $, esc, mm } = ctx;
  const canvas = $('plate-view');
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
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
  const beds = new THREE.Group();
  const pieces = new THREE.Group();
  // The picked piece's outside edges, drawn over it.
  const outline = new THREE.Group();
  const outlineMat = new LineMaterial({ color: 0xffc83d, linewidth: 3 });
  scene.add(beds, pieces, outline);
  const loader = new GLTFLoader();
  let glb = { url: null, parts: null }; // print.glb: piece id → [geometry per part]
  let visible = false;
  let selected = null;
  let only = 0; // 0: every plate; n: just plate n
  let fittedFor = null;
  let drawnKey = null;
  const store = (k, v) => { try { if (v === undefined) return localStorage.getItem(k); localStorage.setItem(k, v); } catch { return null; } };
  let colourBy = store('bid.plates.colour') === 'filament' ? 'filament' : 'app';

  const resize = () => {
    const r = canvas.parentElement.getBoundingClientRect();
    renderer.setSize(r.width, r.height, false);
    outlineMat.resolution.set(r.width, r.height);
    camera.aspect = r.width / Math.max(r.height, 1);
    camera.updateProjectionMatrix();
  };
  new ResizeObserver(resize).observe(canvas.parentElement);
  renderer.setAnimationLoop(() => {
    if (!visible) return;
    controls.update();
    renderer.render(scene, camera);
  });

  const plates = () => ctx.getState()?.plates;
  const cols = () => Math.max(1, Math.ceil(Math.sqrt(plates()?.count || 1)));
  const origin = (n) => {
    const [bx, by] = plates().bed;
    return new THREE.Vector3(((n - 1) % cols()) * bx * SPACING, -Math.floor((n - 1) / cols()) * by * SPACING, 0);
  };
  const plateLabel = (n) => { const name = plates()?.names?.[n - 1]; return name ? `${n} · ${name}` : `Plate ${n}`; };
  const slotColour = (f) => plates()?.filaments?.[f - 1]?.color || '#9b9ea0';

  function dispose(g, keepGeometry) {
    for (const o of [...g.children]) {
      o.traverse((c) => { if (!keepGeometry) c.geometry?.dispose(); if (c.material) [].concat(c.material).forEach((m) => { m.map?.dispose(); m.dispose(); }); });
      g.remove(o);
    }
  }

  function label(text, w) {
    const c = document.createElement('canvas');
    c.width = 1024; c.height = 96;
    const g = c.getContext('2d');
    g.fillStyle = getComputedStyle(document.body).color;
    g.font = '600 56px system-ui, sans-serif';
    g.textBaseline = 'middle';
    g.fillText(text, 8, 48, 1008);
    const tex = new THREE.CanvasTexture(c);
    tex.colorSpace = THREE.SRGBColorSpace;
    return new THREE.Mesh(new THREE.PlaneGeometry(w, w * 96 / 1024), new THREE.MeshBasicMaterial({ map: tex, transparent: true, depthWrite: false }));
  }

  function drawBeds(pl) {
    dispose(beds);
    const [bx, by] = pl.bed;
    for (let n = 1; n <= pl.count; n++) {
      const g = new THREE.Group();
      g.position.copy(origin(n));
      g.userData.plate = n;
      const plate = new THREE.Mesh(new THREE.BoxGeometry(bx, by, 1.5), new THREE.MeshStandardMaterial({ color: 0x3a3d42, roughness: 0.9 }));
      plate.position.set(bx / 2, by / 2, -0.76);
      g.add(plate);
      const grid = new THREE.GridHelper(bx, bx / 16, 0x6b7078, 0x50545b);
      grid.rotation.x = Math.PI / 2;
      grid.position.set(bx / 2, by / 2, 0.02);
      g.add(grid);
      // The margin and the excluded areas, where nothing prints.
      const edge = new THREE.LineSegments(new THREE.EdgesGeometry(new THREE.PlaneGeometry(bx - 2 * pl.margin, by - 2 * pl.margin)), new THREE.LineBasicMaterial({ color: 0x9aa3ad }));
      edge.position.set(bx / 2, by / 2, 0.05);
      g.add(edge);
      for (const [x0, y0, x1, y1] of pl.exclude) {
        const ex = new THREE.Mesh(new THREE.PlaneGeometry(x1 - x0, y1 - y0), new THREE.MeshBasicMaterial({ color: 0xc0392b, transparent: true, opacity: 0.45, depthWrite: false }));
        ex.position.set((x0 + x1) / 2, (y0 + y1) / 2, 0.06);
        g.add(ex);
      }
      // The purge tower's room (brim included), on a plate printing several filaments.
      const tw = pl.towers?.[n - 1];
      if (tw) {
        const [x0, y0, x1, y1] = tw;
        const tower = new THREE.Mesh(new THREE.BoxGeometry(x1 - x0, y1 - y0, 20), new THREE.MeshStandardMaterial({ color: 0xb07cc6, transparent: true, opacity: 0.35, depthWrite: false }));
        tower.position.set((x0 + x1) / 2, (y0 + y1) / 2, 10);
        g.add(tower);
        const tl = label('Purge tower', (x1 - x0) * 1.6);
        tl.position.set((x0 + x1) / 2, (y0 + y1) / 2, 20.5);
        g.add(tl);
      }
      // A plate with a piece that doesn't fit gets a red rim.
      if (pl.pieces.some((p) => p.plate === n && !p.fits)) {
        const rim = new THREE.LineSegments(new THREE.EdgesGeometry(new THREE.PlaneGeometry(bx + 4, by + 4)), new THREE.LineBasicMaterial({ color: 0xe0503c }));
        rim.position.set(bx / 2, by / 2, 0.1);
        g.add(rim);
      }
      const t = label(plateLabel(n), bx * 0.9);
      t.position.set(bx * 0.45, -bx * 0.06, 0);
      g.add(t);
      g.visible = !only || only === n;
      beds.add(g);
    }
  }

  async function printParts(url) {
    if (glb.url === url) return glb.parts;
    const parts = {};
    try {
      const gltf = await loader.loadAsync(url);
      gltf.scene.updateMatrixWorld(true);
      gltf.scene.traverse((o) => {
        if (!o.isMesh || !o.userData?.piece) return;
        (parts[o.userData.piece] ||= [])[o.userData.part] = o.geometry.clone().applyMatrix4(o.matrixWorld);
      });
    } catch { /* not built yet: footprints below */ }
    for (const list of Object.values(glb.parts || {})) for (const g of list) g?.dispose();
    glb = { url, parts };
    return parts;
  }

  function partColour(p, k) {
    if (colourBy === 'filament') return slotColour(p.parts[k]?.filament ?? p.filament);
    // App colours: the piece's own, with parts on another slot than the piece in that slot's.
    const f = p.parts[k]?.filament;
    return f && f !== p.filament ? slotColour(f) : p.color;
  }

  async function drawPieces(s) {
    const pl = s.plates;
    const parts = await printParts(s.print_glb);
    if (plates() !== pl) return; // a newer state came in while loading
    dispose(pieces, true);
    for (const p of pl.pieces) {
      const m = new THREE.Matrix4().makeTranslation(origin(p.plate)).multiply(new THREE.Matrix4().fromArray(p.print_to_plate));
      const geos = parts[p.id];
      const add = (mesh) => {
        mesh.matrixAutoUpdate = false;
        mesh.userData = { piece: p.id, plate: p.plate };
        mesh.visible = !only || only === p.plate;
        pieces.add(mesh);
      };
      if (geos?.length) {
        geos.forEach((g, k) => {
          if (!g) return;
          const mesh = new THREE.Mesh(g, new THREE.MeshStandardMaterial({ color: partColour(p, k), roughness: 0.65, metalness: 0.05 }));
          mesh.matrix.copy(m);
          add(mesh);
        });
      } else {
        // Not built yet: its footprint, so the plate still reads.
        const mesh = new THREE.Mesh(new THREE.BoxGeometry(...p.size), new THREE.MeshStandardMaterial({ color: partColour(p, 0), transparent: true, opacity: 0.5 }));
        mesh.matrix.makeTranslation(origin(p.plate).add(new THREE.Vector3(p.pos[0] + p.size[0] / 2, p.pos[1] + p.size[1] / 2, p.size[2] / 2)));
        add(mesh);
      }
    }
    highlight();
  }

  // The picked piece shows by its outside edges: the outline of what it stands on, top and
  // bottom, and the corners up its sides. Not its pockets, holes or cuts.
  function highlight() {
    for (const o of outline.children) o.geometry.dispose();
    outline.clear();
    for (const m of pieces.children) {
      if (m.userData.piece !== selected || !m.visible) continue;
      const segs = outsideEdges(m.geometry);
      if (!segs.length) continue;
      const line = new LineSegments2(new LineSegmentsGeometry().setPositions(segs), outlineMat);
      line.matrixAutoUpdate = false;
      line.matrix.copy(m.matrix);
      outline.add(line);
    }
  }

  // Outside edges, cached on the geometry: its feature edges whose ends both lie on the outer
  // loop of its bottom face (in plan), as flat [x0,y0,z0, x1,y1,z1, …].
  function outsideEdges(g) {
    if (g.userData.outside) return g.userData.outside;
    const pos = g.attributes.position, idx = g.index;
    const n = idx ? idx.count : pos.count;
    const at = (i) => (idx ? idx.getX(i) : i);
    let zmin = Infinity;
    for (let i = 0; i < pos.count; i++) zmin = Math.min(zmin, pos.getZ(i));
    // The bottom face's edges used once are its boundary.
    const key = (i) => `${Math.round(pos.getX(i) * 1e3)},${Math.round(pos.getY(i) * 1e3)}`;
    const used = new Map();
    const a = new THREE.Vector3(), b = new THREE.Vector3(), c = new THREE.Vector3();
    for (let t = 0; t < n; t += 3) {
      const v = [at(t), at(t + 1), at(t + 2)];
      if (v.some((i) => Math.abs(pos.getZ(i) - zmin) > 0.01)) continue;
      a.fromBufferAttribute(pos, v[0]); b.fromBufferAttribute(pos, v[1]); c.fromBufferAttribute(pos, v[2]);
      for (let k = 0; k < 3; k++) {
        const [p, q] = [v[k], v[(k + 1) % 3]];
        const e = [key(p), key(q)].sort().join('|');
        const u = used.get(e);
        used.set(e, u ? null : [pos.getX(p), pos.getY(p), pos.getX(q), pos.getY(q)]);
      }
    }
    // Join the boundary into loops and keep the one that reaches furthest: the outside.
    const bnd = [...used.values()].filter(Boolean);
    const next = new Map();
    for (const s of bnd) next.set(`${Math.round(s[0] * 1e3)},${Math.round(s[1] * 1e3)}`, s);
    let best = [], bestSpan = -1;
    const seen = new Set();
    for (const s0 of bnd) {
      if (seen.has(s0)) continue;
      const loop = [];
      let s = s0;
      while (s && !seen.has(s)) {
        seen.add(s);
        loop.push(s);
        s = next.get(`${Math.round(s[2] * 1e3)},${Math.round(s[3] * 1e3)}`) || [...next.values()].find((x) => !seen.has(x) && Math.hypot(x[0] - s[2], x[1] - s[3]) < 1e-3);
      }
      const xs = loop.flatMap((q) => [q[0], q[2]]), ys = loop.flatMap((q) => [q[1], q[3]]);
      const span = (Math.max(...xs) - Math.min(...xs)) * (Math.max(...ys) - Math.min(...ys));
      if (span > bestSpan) { bestSpan = span; best = loop; }
    }
    const near = (x, y) => best.some(([x0, y0, x1, y1]) => {
      const dx = x1 - x0, dy = y1 - y0, l2 = dx * dx + dy * dy;
      const t = l2 ? Math.max(0, Math.min(1, ((x - x0) * dx + (y - y0) * dy) / l2)) : 0;
      return Math.hypot(x0 + t * dx - x, y0 + t * dy - y) < 0.05;
    });
    const out = [];
    const e = new THREE.EdgesGeometry(g, 30).attributes.position;
    for (let i = 0; i < e.count; i += 2) {
      if (near(e.getX(i), e.getY(i)) && near(e.getX(i + 1), e.getY(i + 1))) out.push(e.getX(i), e.getY(i), e.getZ(i), e.getX(i + 1), e.getY(i + 1), e.getZ(i + 1));
    }
    g.userData.outside = out;
    return out;
  }

  function fit(force) {
    const pl = plates();
    if (!pl) return;
    const key = `${ctx.game?.()}:${pl.count}:${only}`;
    if (fittedFor === key && !force) return;
    fittedFor = key;
    const [bx, by] = pl.bed;
    let lo, hi;
    if (only) {
      lo = origin(only);
      hi = lo.clone().add(new THREE.Vector3(bx, by, 0));
    } else {
      const rows = Math.ceil(pl.count / cols());
      const c = Math.min(cols(), pl.count);
      lo = new THREE.Vector3(0, -(rows - 1) * by * SPACING, 0);
      hi = new THREE.Vector3(bx + (c - 1) * bx * SPACING, by, 0);
    }
    const w = hi.x - lo.x, d = hi.y - lo.y;
    const c = new THREE.Vector3((lo.x + hi.x) / 2, (lo.y + hi.y) / 2, 20);
    // Far enough back that a sphere round the plates fits the narrower of the two fields of view.
    const R = Math.hypot(w, d, 40) / 2;
    const v = THREE.MathUtils.degToRad(camera.fov) / 2;
    const h = Math.atan(Math.tan(v) * (camera.aspect || 1));
    const dist = (R / Math.sin(Math.min(v, h))) * 0.92;
    camera.position.copy(c).add(new THREE.Vector3(0, -1.15, 1.25).normalize().multiplyScalar(dist));
    controls.target.copy(c);
    camera.near = dist / 100; camera.far = dist * 20; camera.updateProjectionMatrix();
  }

  // Tap a piece to select it; a drag is the camera's.
  const ray = new THREE.Raycaster();
  let down = null;
  canvas.addEventListener('pointerdown', (e) => { down = [e.clientX, e.clientY]; });
  canvas.addEventListener('pointerup', (e) => {
    if (!down || Math.hypot(e.clientX - down[0], e.clientY - down[1]) > 6) return;
    const r = canvas.getBoundingClientRect();
    ray.setFromCamera(new THREE.Vector2(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1), camera);
    const hit = ray.intersectObjects(pieces.children.filter((m) => m.visible), false)[0];
    select(hit ? hit.object.userData.piece : null, true);
  });

  // From the view, bring its row into sight; from the list, leave the page where it is.
  function select(id, fromView) {
    selected = id;
    highlight();
    renderPanel();
    if (id && fromView) document.querySelector(`#plate-list [data-piece="${CSS.escape(id)}"]`)?.scrollIntoView({ block: 'nearest' });
  }

  async function run(body) {
    const v = await ctx.command(body);
    if (v.ok) ctx.toast(v.message);
    // Filament and plate commands answer with the plates as they now are: show them at once
    // rather than after the game's next state, which can wait behind a build.
    const st = ctx.getState();
    if (v.ok && st && v.data?.filaments && v.data?.pieces) { st.plates = v.data; renderPanel(); }
    return v;
  }
  const set = (body) => run({ cmd: 'set_plate', piece: selected, ...body });
  const dims = (a) => a.map(mm).join(' × ') + ' mm';
  const swatch = (c) => `<span class="swatch" style="background:${esc(c)}"></span>`;
  const presetName = (p) => p.replace(/^Bambu /, '').replace(/ @BBL.*$/, '');
  const colourText = (f) => f.colour_name || f.color;
  const slotOptions = (cur) => (plates().filaments || []).map((f, i) => `<option value="${i + 1}" ${cur === i + 1 ? 'selected' : ''}>${i + 1} · ${esc(presetName(f.preset))} ${esc(colourText(f))}</option>`).join('');
  // Bambu's colours for a preset, [name, #RRGGBB]; a slot takes one of them unless "Other" is chosen.
  const bambuColours = (preset) => plates()?.colours?.[preset] || [];
  const hexRgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  // The Bambu colour of `preset` nearest `hex`, for when a slot changes preset.
  const nearest = (preset, hex) => {
    const [r, g, b] = hexRgb(hex);
    let best = null, d = Infinity;
    for (const [, h] of bambuColours(preset)) {
      const [r2, g2, b2] = hexRgb(h), e = (r - r2) ** 2 + (g - g2) ** 2 + (b - b2) ** 2;
      if (e < d) { d = e; best = h; }
    }
    return best;
  };

  function pieceRow(p, checks) {
    const pin = p.pin || {};
    const bits = [dims(p.size)];
    if (p.rotate) bits.push(`turned ${p.rotate}°`);
    if (p.flip) bits.push('upside down');
    const flag = !p.fits ? '<span class="bad">doesn’t fit</span>' : checks[p.id]?.supports ? '<span class="bad">needs supports</span>' : '';
    const placed = pin.plate || pin.rotate != null || pin.flip != null ? '<span class="badge pin" title="Placed by you; the packer arranges the rest">SET</span>' : '';
    const copies = p.copies?.length ? `<span class="badge pin" title="Print once in each: ${esc(p.copies.map((c) => c.name).join(', '))}">×${p.copies.length}</span>` : '';
    const slots = [...new Set([p.filament, ...p.parts.map((x) => x.filament)])];
    return `<button class="plate-piece ${p.id === selected ? 'sel' : ''}" data-piece="${esc(p.id)}">
      ${slots.map((f) => swatch(slotColour(f))).join('')}
      <span class="name">${esc(p.name)} ${copies} ${placed}</span>
      <span class="dims">${bits.map(esc).join(' · ')} ${flag}</span>
    </button>`;
  }

  // The new piece's name as typed: the panel redraws as builds land, and keeps it.
  let newPieceName = '';
  // The spec's piece behind a plate piece, when it was made in SolidCraft.
  const designOf = (p) => p?.kind === 'piece' ? ctx.getState()?.spec?.pieces?.find((x) => `piece/${x.id}` === p.id && x.design) : null;

  function renderPanel() {
    const pl = plates();
    const panel = $('plate-panel');
    if (!pl) { panel.innerHTML = '<section class="card"><p class="muted">Nothing to print yet.</p></section>'; return; }
    const checks = pl.flip_checks || {};
    const sel = pl.pieces.find((p) => p.id === selected);
    if (selected && !sel) selected = null;
    const pinned = pl.pieces.filter((p) => p.pin).length;
    const problems = [...pl.problems.map((p) => `<li class="bad">${esc(p.message)}</li>`), ...(pl.stale || []).map((id) => `<li class="warn">A placement for <code>${esc(id)}</code>, which the layout no longer has</li>`)];
    let html = `<section class="card">
      <h2>Plates <span class="muted small">· ${esc(pl.printer)}</span></h2>
      <div class="fit-grid">
        <div class="stat"><b>${pl.count} plate${pl.count === 1 ? '' : 's'}</b><span>${pl.pieces.length} pieces</span></div>
        <div class="stat"><b class="${pl.problems.length ? 'bad' : 'good'}">${pl.problems.length ? `${pl.problems.length} problem${pl.problems.length === 1 ? '' : 's'}` : 'All fit'}</b><span>${mm(pl.bed[0])} × ${mm(pl.bed[1])} mm bed</span></div>
      </div>
      ${problems.length ? `<ul class="issues">${problems.join('')}</ul>` : ''}
      <p class="muted small">Pieces print the way they sit in the box; a printed box's lid prints top face down. The packer fills each plate, largest first, ${mm(pl.gap)} mm apart, clear of the red corner and, where it can, of the strip along the front beside it; a plate printing several filaments keeps room for its purge tower (purple), which the project puts there; a piece you put on a plate stays there, and a plate that overflows is marked. Tap a piece to turn it, move it, or set its filament.</p>
      ${pinned ? `<button class="toggle" id="plate-reset">Put all ${pinned} back to the packer</button>` : ''}
    </section>`;
    if (sel) {
      const pin = sel.pin || {};
      const n = pl.count;
      const plateBtns = Array.from({ length: n }, (_, i) => i + 1).map((k) => `<button class="toggle ${sel.plate === k ? 'on' : ''}" data-plate="${k}" aria-pressed="${sel.plate === k}" title="${esc(plateLabel(k))}">${k}</button>`).join('');
      const chk = checks[sel.id];
      // Parts get a slot each when there's more than one, or the one has a name of its own.
      const named = sel.parts.length > 1 || (sel.parts[0] && sel.parts[0].name !== sel.name);
      const seen = {};
      const partRows = named ? sel.parts.map((x) => {
        seen[x.name] = (seen[x.name] || 0) + 1;
        if (seen[x.name] > 1) return '';
        return `<label class="row small">${swatch(slotColour(x.filament))} ${esc(x.name)} <select data-part="${esc(x.name)}">${slotOptions(x.filament)}</select></label>`;
      }).join('') : '';
      html += `<section class="card">
        <h2>${esc(sel.name)}</h2>
        <p class="small muted">${dims(sel.size)} on ${esc(plateLabel(sel.plate))}${sel.rotate ? `, turned ${sel.rotate}°` : ''}${sel.flip ? ', upside down' : ''}.</p>
        ${sel.copies?.length ? `<p class="small">Print ${sel.copies.length} times, one in each: ${sel.copies.map((c) => `${swatch(slotColour(c.filament))} ${esc(c.name)}`).join(', ')}.</p>` : ''}
        ${chk?.supports ? `<p class="small bad">Upside down it has ${mm(chk.overhang_mm2)} mm² of overhang flatter than 45°: it would need supports.</p>` : ''}
        ${sel.flip && chk && !chk.checked ? '<p class="small warn">Not checked for supports yet: it hasn’t been built.</p>' : ''}
        <div class="plate-actions">
          <div class="row"><span class="small muted">Plate</span> ${plateBtns} <button class="toggle" data-plate="${n + 1}">New plate</button></div>
          <div class="row">
            <button class="toggle" id="plate-rot">Turn 90°</button>
            <button class="toggle ${sel.flip ? 'on' : ''}" id="plate-flip" aria-pressed="${sel.flip}">Upside down</button>
            ${sel.pin ? '<button class="toggle" id="plate-auto">Back to the packer</button>' : ''}
          </div>
          <label class="row small">Filament <select id="plate-filament">${slotOptions(sel.filament)}</select></label>
          ${partRows ? `<div class="plate-parts"><span class="small muted">Parts</span>${partRows}</div>` : ''}
          ${designOf(sel) ? '<div class="row"><button class="toggle" id="plate-sc-edit">Edit in SolidCraft</button></div>' : ''}
        </div>
        <p class="muted small">${sel.pin && (pin.plate || pin.rotate != null || pin.flip != null) ? `You set its ${['plate', 'rotate', 'flip', 'pos'].filter((k) => pin[k] != null).map((k) => ({ plate: 'plate', rotate: 'turn', flip: 'way up', pos: 'spot' })[k]).join(' and ')}${pin.pos ? '' : '; the packer places it on the plate'}.` : 'The packer chose its plate, turn and place.'}</p>
      </section>`;
    }
    html += `<section class="card"><h2>On each plate</h2><div id="plate-list">`;
    for (let k = 1; k <= pl.count; k++) {
      const on = pl.pieces.filter((p) => p.plate === k);
      html += `<div class="plate-group"><div class="row"><h3 class="small">${esc(plateLabel(k))} <span class="muted">· ${on.length} piece${on.length === 1 ? '' : 's'}</span></h3>
        <button class="toggle small-btn" data-name-plate="${k}">Rename</button></div>${on.map((p) => pieceRow(p, checks)).join('') || '<p class="muted small">Empty</p>'}</div>`;
    }
    html += `</div></section>
      <section class="card">
        <h2>Filament slots</h2>
        <p class="muted small">Slot 1 is the default for every piece. A piece, or a named part of it (a label line), can take another slot.</p>
        <div id="filament-list">${(pl.filaments || []).map((f, i) => `
          <div class="row filament-row">
            <b>${i + 1}</b>
            ${swatch(f.color)}
            <select data-slot-colour="${i + 1}" aria-label="Slot ${i + 1} colour">
              ${bambuColours(f.preset).map(([n, h]) => `<option value="${esc(h)}" ${f.colour_name === n ? 'selected' : ''}>${esc(n)}</option>`).join('')}
              <option value="other" ${f.colour_name ? '' : 'selected'}>${f.colour_name ? 'Other colour…' : `Other: ${esc(f.color)}`}</option>
            </select>
            <input type="color" value="${esc(f.color.toLowerCase())}" data-slot="${i + 1}" aria-label="Slot ${i + 1} other colour" ${f.colour_name ? 'hidden' : ''}>
            <select data-slot-preset="${i + 1}">${(pl.presets || [f.preset]).map((p) => `<option ${p === f.preset ? 'selected' : ''} value="${esc(p)}">${esc(presetName(p))}</option>`).join('')}</select>
            ${pl.filaments.length > 1 ? `<button class="toggle small-btn" data-remove-slot="${i + 1}" aria-label="Remove slot ${i + 1}">Remove</button>` : ''}
          </div>`).join('')}</div>
        <button class="toggle" id="filament-add">Add a slot</button>
      </section>
      <section class="card">
        <h2>New piece in SolidCraft</h2>
        <p class="muted small">Design a piece from scratch in SolidCraft, in a new tab. Each save makes or updates it here, each body a part, as it lies there (standing on the bed). It goes on the plates now; place it in the box from its Tray tab once it's there, or with place_piece.</p>
        <form id="new-piece" class="row"><input name="name" placeholder="Name, e.g. Dice tower" aria-label="New piece name" value="${esc(newPieceName)}" required>
          <button class="primary" type="submit">Open SolidCraft</button></form>
        <p class="small" id="new-piece-status"></p>
      </section>
      <section class="card">
        <h2>Export</h2>
        <p class="muted small">Downloads <code>&lt;game&gt;-plates.3mf</code>, a Bambu Studio project with these plates, their names and the filament slots, X1C 0.4 mm / 0.20 mm Standard. Open it in Bambu Studio and slice.</p>
        <button id="plate-export" class="primary">Export for Bambu Studio</button>
      </section>`;
    panel.innerHTML = html;
    for (const b of panel.querySelectorAll('.plate-piece')) b.addEventListener('click', () => select(b.dataset.piece === selected ? null : b.dataset.piece));
    for (const b of panel.querySelectorAll('[data-plate]')) b.addEventListener('click', () => set({ plate: +b.dataset.plate }));
    for (const b of panel.querySelectorAll('[data-name-plate]')) b.addEventListener('click', () => {
      const k = +b.dataset.namePlate;
      const name = prompt(`Name for plate ${k}`, pl.names?.[k - 1] || '');
      if (name !== null) run({ cmd: 'name_plate', plate: k, name });
    });
    $('plate-rot')?.addEventListener('click', () => set({ rotate: (sel.rotate + 90) % 360 }));
    $('plate-flip')?.addEventListener('click', () => set({ flip: !sel.flip }));
    $('plate-auto')?.addEventListener('click', () => set({ clear: true }));
    $('plate-filament')?.addEventListener('change', (e) => set({ filament: +e.target.value }));
    for (const s of panel.querySelectorAll('[data-part]')) s.addEventListener('change', () => set({ parts: { [s.dataset.part]: +s.value } }));
    for (const i of panel.querySelectorAll('input[data-slot]')) i.addEventListener('change', () => run({ cmd: 'set_filament', slot: +i.dataset.slot, color: i.value }));
    // A Bambu colour goes straight in; "Other" shows the free picker, which sends when it's used.
    for (const s of panel.querySelectorAll('[data-slot-colour]')) s.addEventListener('change', () => {
      if (s.value !== 'other') return run({ cmd: 'set_filament', slot: +s.dataset.slotColour, color: s.value });
      const pick = panel.querySelector(`input[data-slot="${s.dataset.slotColour}"]`);
      pick.hidden = false;
      pick.click();
    });
    // A new preset keeps a Bambu colour: its nearest one, unless the slot was already off the set.
    // The colour as the row shows it now: the panel may not have redrawn since the last pick.
    for (const s of panel.querySelectorAll('[data-slot-preset]')) s.addEventListener('change', () => {
      const cur = panel.querySelector(`[data-slot-colour="${s.dataset.slotPreset}"]`)?.value;
      const color = cur && cur !== 'other' ? nearest(s.value, cur) : null;
      run({ cmd: 'set_filament', slot: +s.dataset.slotPreset, preset: s.value, ...(color ? { color } : {}) });
    });
    for (const b of panel.querySelectorAll('[data-remove-slot]')) b.addEventListener('click', () => run({ cmd: 'remove_filament', slot: +b.dataset.removeSlot }));
    $('filament-add').addEventListener('click', () => run({ cmd: 'set_filament', slot: (pl.filaments?.length || 1) + 1 }));
    $('plate-reset')?.addEventListener('click', () => run({ cmd: 'set_plate', piece: '*', clear: true }));
    $('plate-sc-edit')?.addEventListener('click', () => ctx.openPiece(designOf(sel).id, designOf(sel).name));
    $('new-piece').name.addEventListener('input', (e) => { newPieceName = e.target.value; });
    $('new-piece').addEventListener('submit', (e) => {
      e.preventDefault();
      const name = e.target.name.value.trim();
      if (!name) return;
      ctx.openPiece(null, name);
      newPieceName = '';
      e.target.reset();
      $('new-piece-status').textContent = `Opening SolidCraft for ${name}… it appears here once you save it there.`;
    });
    $('plate-export').addEventListener('click', async (e) => {
      const b = e.target;
      b.disabled = true; b.textContent = 'Exporting…';
      await ctx.onExport();
      b.disabled = false; b.textContent = 'Export for Bambu Studio';
    });
  }

  function showOnly() {
    for (const g of beds.children) g.visible = !only || only === g.userData.plate;
    for (const m of pieces.children) m.visible = !only || only === m.userData.plate;
  }

  function renderPicker() {
    const pl = plates();
    const el = $('plate-pick');
    if (only > (pl?.count || 0)) only = 0;
    const picks = pl && pl.count > 1
      ? `<select id="plate-only" aria-label="Plates shown"><option value="0">All plates</option>${Array.from({ length: pl.count }, (_, i) => `<option value="${i + 1}" ${only === i + 1 ? 'selected' : ''}>${esc(plateLabel(i + 1))}</option>`).join('')}</select>`
      : '';
    el.innerHTML = `${picks}
      <button class="toggle ${colourBy === 'app' ? 'on' : ''}" data-colour="app" aria-pressed="${colourBy === 'app'}">App colours</button>
      <button class="toggle ${colourBy === 'filament' ? 'on' : ''}" data-colour="filament" aria-pressed="${colourBy === 'filament'}">Filament</button>`;
    $('plate-only')?.addEventListener('change', (e) => { only = +e.target.value; showOnly(); fit(true); });
    for (const b of el.querySelectorAll('[data-colour]')) b.addEventListener('click', () => {
      colourBy = b.dataset.colour;
      store('bid.plates.colour', colourBy);
      renderPicker();
      const s = ctx.getState();
      if (s?.plates) drawPieces(s);
    });
  }
  $('plate-reset-view').addEventListener('click', () => fit(true));

  // A new state: redraw what changed.
  async function render() {
    const s = ctx.getState();
    if (!s?.plates) return;
    renderPicker();
    renderPanel();
    const key = JSON.stringify([s.plates.count, s.plates.bed, s.plates.exclude, s.plates.names, s.plates.pieces.map((p) => [p.plate, p.fits])]);
    if (key !== drawnKey) { drawBeds(s.plates); drawnKey = key; }
    await drawPieces(s);
    showOnly();
    if (visible) fit(false);
  }

  return {
    render,
    // Show the plate a piece is on, with the piece picked out.
    focus(id) {
      const p = plates()?.pieces.find((x) => x.id === id);
      if (!p) return;
      only = p.plate;
      renderPicker();
      showOnly();
      select(id, false);
      fittedFor = null;
    },
    show(on) {
      visible = on;
      if (on) { requestAnimationFrame(() => { resize(); fit(false); }); }
    },
  };
}
