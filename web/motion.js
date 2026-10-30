// Motion video: the box opens, each tray goes out to the table one at a time, and (optionally)
// everything packs back in. Plays in the viewer, and exports to an MP4 without playing: each
// frame is drawn off screen and encoded (WebCodecs), the view put back between frames. It moves
// the viewer's own meshes and puts every one back when it stops.

import * as THREE from 'three';
import { Muxer, ArrayBufferTarget } from 'https://cdn.jsdelivr.net/npm/mp4-muxer@5.1.3/build/mp4-muxer.mjs';

const KEY = 'bid-motion';
const DEFAULTS = { speed: 1, pause: 0.4, repack: true, reverse: false, contents: true, art: true, camera: 'orbit', table: true, loop: false, size: '1280x720' };
const SIZES = { '1280x720': [1280, 720], '1920x1080': [1920, 1080], '1080x1080': [1080, 1080] };
const MIMES = ['video/mp4;codecs=avc1', 'video/mp4', 'video/webm;codecs=vp9', 'video/webm'];
const GAP = 15;
const FPS = 30;

const load = () => { try { return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(KEY) || '{}') }; } catch { return { ...DEFAULTS }; } };
const save = (o) => { try { localStorage.setItem(KEY, JSON.stringify(o)); } catch { /* private mode: options last this visit */ } };
const ease = (u) => (u < 0.5 ? 4 * u * u * u : 1 - (-2 * u + 2) ** 3 / 2);
const lerp = (a, b, u) => a.clone().lerp(b, u);

// ctx: the viewer's scene, camera, controls, renderer, groups, and its helpers (see app.js).
export function initMotion(ctx) {
  const opts = load();
  const group = new THREE.Group();
  ctx.scene.add(group);
  let run = null; // { segs, total, t0, pieces, saved, cam, rec, center, radius }

  // ---------- panel ----------
  const panel = document.getElementById('motion-panel');
  const sel = (name, pairs) => `<select name="${name}">${pairs.map(([v, t]) => `<option value="${v}" ${String(opts[name]) === String(v) ? 'selected' : ''}>${t}</option>`).join('')}</select>`;
  const chk = (name, label) => `<label class="check"><input type="checkbox" name="${name}" ${opts[name] ? 'checked' : ''}> ${label}</label>`;
  panel.innerHTML = `
    <div class="motion-head"><b>Motion</b><button class="toggle" data-act="close" aria-label="Close">✕</button></div>
    <form class="motion-form">
      <label>Speed ${sel('speed', [[0.6, 'Slow'], [1, 'Normal'], [1.6, 'Fast']])}</label>
      <label>Pause <input name="pause" type="range" min="0" max="2" step="0.1" value="${opts.pause}"><output>${opts.pause} s</output></label>
      <label>Camera ${sel('camera', [['orbit', 'Slow orbit'], ['still', 'Still']])}</label>
      ${chk('repack', 'Pack it back')}${chk('reverse', 'Start on the table')}${chk('contents', 'Contents')}${chk('art', 'Box art')}${chk('table', 'Table')}${chk('loop', 'Loop')}
      <label>Video ${sel('size', [['1280x720', '1280 × 720'], ['1920x1080', '1920 × 1080'], ['1080x1080', '1080 × 1080']])}</label>
    </form>
    <div class="motion-actions">
      <button class="primary" data-act="play">Play</button>
      <button class="toggle" data-act="export">Export video</button>
    </div>
    <div class="motion-status muted small"></div>`;
  const form = panel.querySelector('form');
  const status = panel.querySelector('.motion-status');
  const playBtn = panel.querySelector('[data-act=play]');
  const exportBtn = panel.querySelector('[data-act=export]');
  form.addEventListener('input', () => {
    const f = form.elements;
    Object.assign(opts, {
      speed: +f.speed.value, pause: +f.pause.value, camera: f.camera.value, size: f.size.value,
      repack: f.repack.checked, reverse: f.reverse.checked, contents: f.contents.checked, art: f.art.checked, table: f.table.checked, loop: f.loop.checked,
    });
    form.querySelector('output').textContent = `${opts.pause} s`;
    save(opts);
  });
  const encodable = typeof VideoEncoder !== 'undefined' && typeof VideoFrame !== 'undefined';
  const recordable = typeof MediaRecorder !== 'undefined' && !!ctx.canvas.captureStream;
  if (!encodable && !recordable) { exportBtn.disabled = true; status.textContent = 'This browser can’t make video files: play only.'; }
  panel.addEventListener('click', (e) => {
    const act = e.target.closest('[data-act]')?.dataset.act;
    if (act === 'close') { stop(); panel.hidden = true; ctx.button.classList.remove('on'); }
    if (act === 'play') { if (run) stop(); else start(false); }
    if (act === 'export') {
      if (exporting) exporting.cancel = true;
      else if (run?.rec) stop();
      else { if (run) stop(); exportVideo(); }
    }
  });
  ctx.button.addEventListener('click', () => {
    const only = ctx.onlyTray?.();
    const t = only && ctx.getState()?.layout?.trays.find((x) => x.id === only);
    panel.querySelector('.motion-head b').textContent = t ? `Motion · ${t.name}` : 'Motion';
    panel.hidden = !panel.hidden;
    ctx.button.classList.toggle('on', !panel.hidden);
    if (panel.hidden) stop();
  });

  // ---------- the pieces and their moves ----------
  function bounds(objs) {
    const b = new THREE.Box3();
    for (const o of objs) b.expandByObject(o);
    return b;
  }

  function build() {
    const box = ctx.curBox();
    const state = ctx.getState();
    const L = state?.layout;
    if (!box || !L) return null;
    const meshes = ctx.modelGroup.children;
    const pieces = [];
    const add = (objs, kind, label, tray) => { if (objs.length) pieces.push({ objs, kind, label, tray }); };

    // The box: a bought one drawn as base + telescoping lid; a printed one is in the model.
    let base = null;
    let lid = null;
    if (!box.printed) {
      const bc = state.spec.components.find((c) => c.id === box.id);
      if (bc) {
        const [Li, Wi] = box.size.inner;
        const g = ctx.boxShapes(bc, Li >= Wi);
        const plain = () => new THREE.MeshStandardMaterial({ color: 0xb3aa96, side: THREE.DoubleSide });
        const hidden = new THREE.MeshStandardMaterial({ visible: false });
        base = new THREE.Mesh(g.base, g.baseMats);
        base.position.set(Li / 2, Wi / 2, g.baseZ);
        const lidMats = opts.art ? g.lidMats : [plain(), plain(), plain(), plain(), plain(), hidden];
        lid = new THREE.Mesh(g.lid, lidMats);
        const lift = L.report.lid_lift?.mm || 0;
        lid.position.set(Li / 2, Wi / 2, g.oH - g.floor + lift - g.skirt / 2);
        group.add(base, lid);
        add([lid], 'lid', 'Lid');
      }
    } else {
      const pl = meshes.find((m) => m.userData.kind === 'printed_lid');
      if (pl) add([pl], 'printed_lid', 'Lid');
    }

    // What lies on top: topmost first.
    const nl = L.layers.length;
    for (const o of [...(L.on_top || [])].reverse()) {
      add(meshes.filter((m) => m.userData.kind === 'ghost' && m.userData.component === o.component && (m.userData.layer ?? 0) >= nl), 'on_top', o.label);
    }

    // Trays: top layer first, back to front, left to right. A frame of holders stays in the
    // box: its holders come out of it.
    const trays = [...L.trays].sort((a, b) => b.layer - a.layer || a.pos[1] - b.pos[1] || a.pos[0] - b.pos[0]);
    for (const t of trays) {
      if (t.frame) continue;
      add(meshes.filter((m) => m.userData.tray === t.id && (m.userData.kind === 'tray' || m.userData.kind === 'label' || m.userData.kind === 'ghost')), t.holder_of ? 'holder' : 'tray', t.name, [t.id, t.holder_of]);
    }

    // Where the box is, and the table under it.
    const fixed = base ? [base] : meshes.filter((m) => m.userData.kind === 'printed_box');
    const rest = bounds([...fixed, ...pieces.flatMap((p) => p.objs)]);
    const boxB = fixed.length ? bounds(fixed) : new THREE.Box3(new THREE.Vector3(0, 0, 0), new THREE.Vector3(...box.size.inner));
    const tableZ = boxB.min.z;
    const clearZ = rest.max.z + 20;
    const width = boxB.max.y - boxB.min.y;

    // Table spots: the lid to the left, the rest in columns to the right, in removal order, but
    // what lay on top (a board) goes last: it's big and flat, and the trays sit nearer the box.
    let x = boxB.max.x + 40;
    let y = boxB.min.y;
    let col = 0;
    const far = new THREE.Box3().copy(rest);
    for (const p of [...pieces.filter((q) => q.kind !== 'on_top'), ...pieces.filter((q) => q.kind === 'on_top')]) {
      p.b = bounds(p.objs);
      const sx = p.b.max.x - p.b.min.x;
      const sy = p.b.max.y - p.b.min.y;
      let at;
      if (p.kind === 'lid' || p.kind === 'printed_lid') {
        at = new THREE.Vector3(boxB.min.x - 40 - sx, boxB.min.y, tableZ);
      } else {
        if (y > boxB.min.y && y + sy > boxB.min.y + 1.4 * width) { x += col + GAP; y = boxB.min.y; col = 0; }
        at = new THREE.Vector3(x, y, tableZ);
        y += sy + GAP;
        col = Math.max(col, sx);
      }
      p.to = new THREE.Vector3(at.x - p.b.min.x, at.y - p.b.min.y, at.z - p.b.min.z);
      far.expandByPoint(at).expandByPoint(new THREE.Vector3(at.x + sx, at.y + sy, at.z));
    }

    if (opts.table) {
      const pad = 60;
      const s = far.getSize(new THREE.Vector3());
      const c = far.getCenter(new THREE.Vector3());
      const table = new THREE.Mesh(new THREE.PlaneGeometry(s.x + 2 * pad, s.y + 2 * pad), new THREE.MeshStandardMaterial({ color: 0xcbb898, roughness: 0.9 }));
      table.position.set(c.x, c.y, tableZ - 0.3);
      group.add(table);
    }

    // The timeline: per piece, (slide,) lift, travel on an arc, lower; then the same in reverse.
    const sp = opts.speed;
    const segs = [];
    let t = 0;
    const seg = (pi, dur, fn) => { segs.push({ pi, start: t, dur, fn }); t += dur; };
    // Moves that run side by side: each starts now, and the timeline goes on after the longest.
    const together = (list) => {
      const t0 = t;
      let end = t0;
      for (const [pi, dur, fn] of list) { segs.push({ pi, start: t0, dur, fn }); end = Math.max(end, t0 + dur); }
      t = end;
    };
    const wait = (dur) => { if (dur > 0) seg(-1, dur, null); };
    const outMoves = [];
    for (const [pi, p] of pieces.entries()) {
      const moves = [];
      let at = new THREE.Vector3();
      if (p.kind === 'printed_lid') {
        const slid = new THREE.Vector3(p.b.max.x - p.b.min.x + 10, 0, 0);
        moves.push([0.9, at, slid, 0]);
        at = slid;
      }
      const up = new THREE.Vector3(at.x, at.y, at.z + Math.max(0, clearZ - p.b.min.z));
      if (p.kind === 'lid') up.z = at.z + 60;
      const over = new THREE.Vector3(p.to.x, p.to.y, up.z);
      moves.push([0.7, at, up, 0], [0.6 + up.distanceTo(over) / 450, up, over, 25], [0.7, over, p.to, 0]);
      outMoves.push(moves);
    }
    // The deck holders are small and alike: they go quicker, with a shorter pause.
    const quick = (pi) => (pieces[pi].kind === 'holder' ? 0.45 : 1);
    // On the Tray tab, just that tray's part (a frame's: its holders): what comes out before it
    // is already on the table, what comes after stays in the box.
    const only = ctx.onlyTray?.();
    const mine = pieces.map((p) => !only || (p.tray || []).includes(only));
    const firstMine = mine.indexOf(true);
    if (firstMine < 0) mine.fill(true);
    const arcAt = (arc, u) => new THREE.Vector3(0, 0, arc * Math.sin(Math.PI * u));
    // What moves as one: a frame's holders come out (and go back) together; the rest one by one.
    const units = [];
    for (const [pi, p] of pieces.entries()) {
      if (!mine[pi]) continue;
      const frame = p.kind === 'holder' ? p.tray?.[1] : null;
      const u = frame && units.find((x) => x.frame === frame);
      if (u) u.pis.push(pi); else units.push({ frame, pis: [pi] });
    }
    const steps = (pis, rev) => {
      const n = Math.max(...pis.map((pi) => outMoves[pi].length));
      for (let k = 0; k < n; k++) {
        together(pis.flatMap((pi) => {
          const moves = rev ? [...outMoves[pi]].reverse() : outMoves[pi];
          if (!moves[k]) return [];
          const [d, a, b, arc] = moves[k];
          return [[pi, (d * quick(pi)) / sp, rev ? (u) => lerp(b, a, u).add(arcAt(arc, u)) : (u) => lerp(a, b, u).add(arcAt(arc, u))]];
        }));
      }
      wait((opts.pause * quick(pis[0])) / sp);
    };
    const out = () => units.forEach((u) => steps(u.pis, false));
    const back = () => [...units].reverse().forEach((u) => steps([...u.pis].reverse(), true));
    // Starting on the table: everything is laid out, packs in, and (with Pack it back) comes out again.
    const [first, then] = opts.reverse ? [back, out] : [out, back];
    // One tray's part holds a while where it lands (on the table, or back in the box), so it
    // can be looked at before it goes back, or before the loop starts over.
    const hold = (only ? 2.5 + opts.pause : 1.2) / sp;
    if (opts.reverse) wait(0.8 / sp);
    first();
    if (opts.repack) { wait(hold); then(); }
    wait(only ? hold : 0.8 / sp);
    // Where each piece rests before its first move.
    pieces.forEach((p, pi) => {
      const onTable = mine[pi] ? opts.reverse : pi < firstMine;
      p.rest = onTable ? p.to.clone() : new THREE.Vector3();
    });

    const view = new THREE.Box3().copy(far).union(rest);
    return { pieces, segs, total: t, view };
  }

  // ---------- play ----------
  function start(record) {
    const plan = build();
    if (!plan) { status.textContent = 'Nothing to show yet.'; return; }
    const saved = new Map();
    for (const p of plan.pieces) for (const o of p.objs) saved.set(o, o.position.clone());
    // The model as it is in the box: every layer, nothing lifted, the wireframe out of the way.
    for (const m of ctx.modelGroup.children) {
      const u = m.userData;
      m.visible = !u.replaced && (u.kind !== 'ghost' || opts.contents);
      if (u.kind === 'printed_lid' || u.kind === 'printed_box') m.visible = true;
      if (u.kind === 'tray' || u.kind === 'label') for (const mat of [].concat(m.material)) { mat.transparent = false; mat.opacity = 1; mat.depthWrite = true; }
      if (u.baseZ !== undefined && !saved.has(m)) m.position.z = u.baseZ;
    }
    for (const p of plan.pieces) for (const o of p.objs) if (o.userData.baseZ !== undefined) { o.position.z = o.userData.baseZ; saved.set(o, o.position.clone()); }
    for (const c of ctx.boxGroup.children) c.visible = false;
    ctx.boxGroup.visible = false;
    const center = plan.view.getCenter(new THREE.Vector3());
    // Far enough that the box and the whole table stay in frame as the camera circles.
    const radius = plan.view.getSize(new THREE.Vector3()).length() * 0.95;
    const cam = { pos: ctx.camera.position.clone(), target: ctx.controls.target.clone(), enabled: ctx.controls.enabled, aspect: ctx.camera.aspect };
    run = { ...plan, saved, cam, center, radius, t0: performance.now(), rec: null };
    ctx.controls.target.copy(center);
    ctx.controls.enabled = opts.camera !== 'orbit';
    placeCamera(0);
    ctx.camera.near = radius / 100; ctx.camera.far = radius * 30; ctx.camera.updateProjectionMatrix();
    if (record) startRecording();
    playBtn.textContent = 'Stop';
    panel.classList.add('playing');
    ctx.canvas.parentElement.classList.add('motion-playing');
    exportBtn.textContent = record ? 'Stop recording' : 'Export video';
  }

  function placeCamera(t) {
    if (opts.camera !== 'orbit' && t > 0) return;
    aim(ctx.camera, run.center, run.radius, t);
  }

  function aim(camera, c, r, t) {
    const a = -Math.PI * 0.62 + (opts.camera === 'orbit' ? t * 0.06 : 0);
    camera.position.set(c.x + r * Math.cos(a), c.y + r * Math.sin(a), c.z + r * 0.75);
    camera.lookAt(c);
  }

  // Each piece's offset from where it sits in the box, at time t.
  function offsets(plan, t) {
    const off = plan.pieces.map((p) => p.rest.clone());
    for (const s of plan.segs) {
      if (s.pi < 0) continue;
      if (t >= s.start + s.dur) off[s.pi] = s.fn(1);
      // Segments run in start order; several may run at once (a frame's holders move together).
      else if (t >= s.start) off[s.pi] = s.fn(ease((t - s.start) / s.dur));
      else break;
    }
    return off;
  }

  function startRecording() {
    const [w, h] = SIZES[opts.size] || SIZES['1280x720'];
    const mime = MIMES.find((m) => MediaRecorder.isTypeSupported(m)) || '';
    run.px = ctx.renderer.getPixelRatio();
    ctx.renderer.setPixelRatio(1);
    ctx.renderer.setSize(w, h, false);
    ctx.camera.aspect = w / h;
    ctx.camera.updateProjectionMatrix();
    const chunks = [];
    const rec = new MediaRecorder(ctx.canvas.captureStream(30), mime ? { mimeType: mime, videoBitsPerSecond: w * h * 6 } : undefined);
    rec.ondataavailable = (e) => { if (e.data.size) chunks.push(e.data); };
    const type = mime || rec.mimeType || 'video/webm';
    rec.onstop = () => {
      const blob = new Blob(chunks, { type });
      if (!blob.size) { status.textContent = 'The recording came out empty.'; return; }
      const a = document.createElement('a');
      a.href = URL.createObjectURL(blob);
      a.download = `${ctx.game() || 'insert'}-${ctx.onlyTray?.() || 'unboxing'}.${type.includes('mp4') ? 'mp4' : 'webm'}`;
      document.body.append(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(a.href), 60000);
      status.textContent = `Saved ${a.download} (${(blob.size / 1e6).toFixed(1)} MB).`;
    };
    rec.start(1000);
    run.rec = rec;
  }

  // Called every frame from the viewer's render loop.
  function update(now) {
    if (!run) return;
    const t = (now - run.t0) / 1000;
    const off = offsets(run, t);
    run.pieces.forEach((p, i) => { for (const o of p.objs) o.position.copy(run.saved.get(o)).add(off[i]); });
    placeCamera(t);
    status.textContent = `${run.rec ? 'Recording' : 'Playing'} · ${Math.min(t, run.total).toFixed(1)} / ${run.total.toFixed(1)} s`;
    if (t >= run.total) {
      if (opts.loop && !run.rec) { run.t0 = now; return; }
      stop();
    }
  }

  // Put everything back as the viewer had it.
  function stop() {
    if (!run) return;
    const r = run;
    run = null;
    if (r.rec && r.rec.state !== 'inactive') r.rec.stop();
    for (const [o, p] of r.saved) o.position.copy(p);
    disposeAll();
    if (r.px) ctx.renderer.setPixelRatio(r.px);
    ctx.resize();
    ctx.camera.position.copy(r.cam.pos);
    ctx.controls.target.copy(r.cam.target);
    ctx.controls.enabled = r.cam.enabled;
    ctx.camera.lookAt(r.cam.target);
    if (ctx.curBox() && ctx.getState()?.layout) ctx.drawBox(ctx.curBox(), ctx.getState().layout);
    ctx.applyView();
    playBtn.textContent = 'Play';
    panel.classList.remove('playing');
    ctx.canvas.parentElement.classList.remove('motion-playing');
    exportBtn.textContent = 'Export video';
    if (!r.rec) status.textContent = '';
    else if (!status.textContent.startsWith('Saved')) status.textContent = 'Saving the video…';
  }

  // ---------- export, without playing ----------
  // Every frame at its own time, drawn by a renderer of its own at the video's size and encoded
  // as H.264 into an MP4. The scene is set for the frame and put back before anything else can
  // draw it, so the view on screen never changes. Where the browser has no WebCodecs (or no
  // H.264 encoder), it falls back to recording the screen as it plays.
  let exporting = null;
  async function exportVideo() {
    const [w, h] = SIZES[opts.size] || SIZES['1280x720'];
    const cfg = { codec: 'avc1.640028', width: w, height: h, bitrate: w * h * 6, framerate: FPS };
    const ok = encodable && (await VideoEncoder.isConfigSupported(cfg).catch(() => ({ supported: false }))).supported;
    if (!ok) { if (recordable) start(true); else status.textContent = 'This browser can’t make video files.'; return; }
    const plan = build();
    if (!plan) { status.textContent = 'Nothing to show yet.'; return; }
    const job = { cancel: false };
    exporting = job;
    exportBtn.textContent = 'Cancel export';
    playBtn.disabled = true;

    // The view as it is, to put back after each frame.
    const meshes = ctx.modelGroup.children;
    const kept = meshes.map((m) => [m, m.visible, m.position.clone(), [].concat(m.material).map((x) => [x, x.transparent, x.opacity, x.depthWrite])]);
    const boxKept = [ctx.boxGroup.visible, ...ctx.boxGroup.children.map((c) => c.visible)];
    const home = new Map();
    for (const p of plan.pieces) for (const o of p.objs) home.set(o, o.userData.baseZ !== undefined ? new THREE.Vector3(o.position.x, o.position.y, o.userData.baseZ) : o.position.clone());
    group.visible = false;
    const frameOn = (t) => {
      for (const m of meshes) {
        const u = m.userData;
        m.visible = !u.replaced && (u.kind !== 'ghost' || opts.contents);
        if (u.kind === 'printed_lid' || u.kind === 'printed_box') m.visible = true;
        if (u.kind === 'tray' || u.kind === 'label') for (const mat of [].concat(m.material)) { mat.transparent = false; mat.opacity = 1; mat.depthWrite = true; }
        if (u.baseZ !== undefined && !home.has(m)) m.position.z = u.baseZ;
      }
      const off = offsets(plan, t);
      plan.pieces.forEach((p, i) => { for (const o of p.objs) o.position.copy(home.get(o)).add(off[i]); });
      ctx.boxGroup.visible = false;
      group.visible = true;
    };
    const frameOff = () => {
      for (const [m, v, pos, mats] of kept) {
        m.visible = v;
        m.position.copy(pos);
        for (const [x, tr, op, dw] of mats) { x.transparent = tr; x.opacity = op; x.depthWrite = dw; }
      }
      ctx.boxGroup.visible = boxKept[0];
      ctx.boxGroup.children.forEach((c, i) => { c.visible = boxKept[i + 1]; });
      group.visible = false;
    };

    const center = plan.view.getCenter(new THREE.Vector3());
    const radius = plan.view.getSize(new THREE.Vector3()).length() * 0.95;
    const camera = new THREE.PerspectiveCamera(ctx.camera.fov, w / h, radius / 100, radius * 30);
    const r = new THREE.WebGLRenderer({ antialias: true, preserveDrawingBuffer: true });
    r.setPixelRatio(1);
    r.setSize(w, h, false);
    r.outputColorSpace = ctx.renderer.outputColorSpace;
    r.toneMapping = ctx.renderer.toneMapping;
    // The viewer's canvas is see-through over the page; a video needs that page colour drawn in.
    let bg = null;
    for (let el = ctx.canvas; el && !bg; el = el.parentElement) {
      const c = getComputedStyle(el).backgroundColor;
      if (c && c !== 'transparent' && !/rgba\(.*,\s*0\)$/.test(c)) bg = c;
    }
    r.setClearColor(new THREE.Color(bg || getComputedStyle(document.body).backgroundColor || '#e9e7e2'), 1);

    const muxer = new Muxer({ target: new ArrayBufferTarget(), video: { codec: 'avc', width: w, height: h, frameRate: FPS }, fastStart: 'in-memory' });
    let failed = null;
    const enc = new VideoEncoder({ output: (chunk, meta) => muxer.addVideoChunk(chunk, meta), error: (e) => { failed = e; } });
    enc.configure(cfg);
    const frames = Math.ceil(plan.total * FPS) + 1;
    try {
      for (let i = 0; i < frames && !job.cancel && !failed; i++) {
        const t = i / FPS;
        frameOn(t);
        aim(camera, center, radius, opts.camera === 'orbit' ? t : 0);
        r.render(ctx.scene, camera);
        frameOff();
        const vf = new VideoFrame(r.domElement, { timestamp: Math.round((i * 1e6) / FPS), duration: Math.round(1e6 / FPS) });
        enc.encode(vf, { keyFrame: i % (2 * FPS) === 0 });
        vf.close();
        // Let the page breathe, and the encoder catch up.
        if (i % 4 === 3 || enc.encodeQueueSize > 8) {
          status.textContent = `Exporting · ${Math.round((100 * i) / frames)}%`;
          while (enc.encodeQueueSize > 8) await new Promise((res) => setTimeout(res, 5));
          await new Promise((res) => setTimeout(res, 0));
        }
      }
      if (failed) throw failed;
      if (job.cancel) { status.textContent = 'Export cancelled.'; return; }
      await enc.flush();
      muxer.finalize();
      const blob = new Blob([muxer.target.buffer], { type: 'video/mp4' });
      const a = document.createElement('a');
      a.href = URL.createObjectURL(blob);
      a.download = `${ctx.game() || 'insert'}-${ctx.onlyTray?.() || 'unboxing'}.mp4`;
      document.body.append(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(a.href), 60000);
      status.textContent = `Saved ${a.download} (${(blob.size / 1e6).toFixed(1)} MB, ${plan.total.toFixed(1)} s).`;
    } catch (e) {
      status.textContent = `The export failed: ${e.message || e}`;
    } finally {
      frameOff();
      if (enc.state !== 'closed') enc.close();
      r.dispose();
      disposeAll();
      group.visible = true;
      exporting = null;
      exportBtn.textContent = 'Export video';
      playBtn.disabled = false;
    }
  }

  function disposeAll() {
    group.traverse((o) => { if (o !== group) { o.geometry?.dispose(); } });
    group.clear();
  }

  return { update, stop, playing: () => !!run };
}
