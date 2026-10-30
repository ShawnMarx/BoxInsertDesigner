// Edit in SolidCraft: SolidCraft's web app (the pinned release, served at /solidcraft/) in a new
// tab, holding a tray with its full history (every box, cut, sketch and fillet editable), a piece
// made there, or a new design for a piece.
//
// While the tab is open this page follows the design (`document.revision`); a few seconds after
// the edits pause it saves the design into the app's own browser storage (`file.save_as` to
// /opfs/…), reads the file back from there (same origin: OPFS, or local storage where the page
// isn't https) and sends it to the server. A tray's edit is compared with the generated tray
// (`submit_edit`), and kept as the tray only when asked (`keep_edit`); a piece's design becomes
// the piece (`import_design`).

const POLL_MS = 1500;
// Edits this long apart are one: the design is saved once they pause.
const SETTLE_MS = 4000;
const READY_MS = 180000;
// How long the app may take to start on WebGPU before it is tried with WebGL2.
const WEBGPU_MS = 45000;

// Open editors, by game/key (a tray's id, or piece:<id>): { w, status, timer, … }.
const sessions = new Map();
let ui = { command: null, onStatus: () => {}, onPiece: () => {} };

// Whether WebGPU really works here: some browsers have navigator.gpu but no adapter, and the app
// then never starts. Asked once, ahead of the click that has to open the tab at once.
let webgpu = false;
navigator.gpu?.requestAdapter().then((a) => { webgpu = !!a; }, () => {});

export function initSolidcraft(opts) { ui = { ...ui, ...opts }; }

export function editorStatus(game, key) { return sessions.get(`${game}/${key}`)?.status || ''; }

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function say(s, text) {
  s.status = text;
  ui.onStatus(s.game, s.key, text);
}

// The file the editor saves the design to, in its browser storage.
const fileName = (s) => `bid-${s.game}-${s.key}.sc`.replace(/[^A-Za-z0-9._-]/g, '-');

// A new tab for the app, or the one already open for `key`. Call from the click itself, so the
// browser lets the new tab open.
function open(game, key, fields) {
  const id = `${game}/${key}`;
  const old = sessions.get(id);
  if (old && !old.w.closed) { old.w.focus(); return null; }
  // WebGPU needs https and a GPU adapter; without them the app draws with WebGL2.
  const url = `/solidcraft/${window.isSecureContext && webgpu ? '' : '?webgl'}`;
  const w = window.open(url, `solidcraft-${id}`);
  if (!w) { ui.onStatus(game, key, 'The browser blocked the new tab: allow pop-ups for this page.'); return null; }
  const s = { game, key, w, status: '', timer: null, ...fields };
  sessions.set(id, s);
  return s;
}

// A tray: the generated one with its history, or the edit kept as it.
export function openEditor(game, box, tray, trayName) {
  const s = open(game, tray, { kind: 'tray', box, tray, title: trayName });
  if (s) load(s).catch((e) => say(s, `Couldn't load the tray into SolidCraft: ${e.message || e}`));
}

// A piece made in SolidCraft (`piece`), or a new one called `name`.
export function openPiece(game, piece, name) {
  const key = piece ? `piece:${piece}` : `piece:new-${Date.now()}`;
  const s = open(game, key, { kind: 'piece', piece, title: name || piece || 'New piece' });
  if (s) load(s).catch((e) => say(s, `Couldn't open the piece in SolidCraft: ${e.message || e}`));
}

async function ready(s) {
  const t0 = Date.now();
  // window.solidcraft appears once the app has started; it answers once its first frame is up.
  for (;;) {
    if (s.w.closed) return false;
    try { if (s.w.solidcraft) { await s.w.solidcraft.request('ui.inspect'); return true; } } catch { /* not yet */ }
    // A GPU adapter that can't run the app leaves it stuck: try again with WebGL2.
    if (Date.now() - t0 > WEBGPU_MS && !s.w.location.search.includes('webgl')) {
      say(s, 'Starting SolidCraft with WebGL…');
      s.w.location.search = '?webgl';
    }
    if (Date.now() - t0 > READY_MS) throw new Error('SolidCraft didn’t start (it needs WebGPU or WebGL2)');
    await sleep(500);
  }
}

async function load(s) {
  say(s, 'Starting SolidCraft…');
  // What goes in: a tray's script (or its kept edit's history), a piece's history, or nothing.
  let steps = null, fallback = null, what = 'the tray';
  if (s.kind === 'tray') {
    const v = await ui.command({ cmd: 'edit_script', box: s.box, tray: s.tray });
    if (!v.ok) throw new Error(v.reason?.message || 'no script');
    s.kept = !!v.data.design_commands;
    steps = v.data.design_commands || v.data.commands;
    fallback = v.data.design_commands ? null : v.data.commands_no_scoops;
    if (s.kept) what = 'your kept edit';
  } else if (s.piece) {
    const v = await ui.command({ cmd: 'piece_design', piece: s.piece });
    if (!v.ok) throw new Error(v.reason?.message || 'no design');
    steps = v.data.commands;
    what = 'the piece';
  }
  if (!(await ready(s))) return say(s, '');
  const sc = s.w.solidcraft;
  await sc.request('ui.home', { open: false });
  await sc.execute('file.new', { name: s.title });
  if (steps) {
    say(s, `Building ${what}…`);
    let r = await sc.request('engine.batch', { commands: steps });
    // As the server's build does: without scoops if the kernel refuses one (the batch rolled back).
    if (r.failed && fallback && r.failed.command === 'solid.fillet') r = await sc.request('engine.batch', { commands: fallback });
    if (r.failed) throw new Error(`step ${r.failed.index + 1} (${r.failed.command}): ${r.failed.error}`);
    await sc.request('ui.view', { view: 'iso' });
    await sc.request('ui.camera', { fit: true });
  }
  watch(s, (await sc.request('document.revision')).revision);
}

// Follow the design on the editor tab's own timers: this tab is hidden while the user edits there,
// and a hidden tab's timers are slowed to once a minute.
function watch(s, base) {
  let seen = base, saved = base, changed = 0, busy = false;
  say(s, s.kind === 'tray'
    ? 'Open in SolidCraft. Edits are compared with the generated tray as you make them.'
    : 'Open in SolidCraft. Each body is a part of the piece; it updates here as you go.');
  const tick = async () => {
    if (s.w.closed) {
      clearInterval(s.timer);
      sessions.delete(`${s.game}/${s.key}`);
      return say(s, seen !== saved ? 'SolidCraft closed before the last edits were saved.' : '');
    }
    if (busy) return;
    busy = true;
    try {
      const { revision } = await s.w.solidcraft.request('document.revision');
      if (revision !== seen) { seen = revision; changed = Date.now(); say(s, 'Editing…'); }
      else if (seen !== saved && Date.now() - changed > SETTLE_MS) {
        const tray = s.kind === 'tray';
        say(s, tray ? 'Comparing with the generated tray…' : 'Updating the piece…');
        await save(s);
        saved = seen = (await s.w.solidcraft.request('document.revision')).revision;
        say(s, `${tray ? 'Compared' : 'Piece updated'} at ${new Date().toLocaleTimeString()}. Keep editing; it updates as you go.`);
      }
    } catch (e) {
      say(s, `Couldn't follow the edit: ${e.message || e}`);
    } finally { busy = false; }
  };
  s.timer = s.w.setInterval(tick, POLL_MS);
}

async function save(s) {
  const name = fileName(s);
  try { localStorage.removeItem(`solidcraft.file.${name}`); } catch { /* none */ }
  const t0 = Date.now();
  await s.w.solidcraft.execute('file.save_as', { path: `/opfs/${name}`, cache: false, overwrite: true });
  const bytes = await readBack(name, t0);
  if (s.kind === 'tray') {
    const v = await ui.command({ cmd: 'submit_edit', box: s.box, tray: s.tray, design: toBase64(bytes) });
    if (!v.ok) throw new Error(v.reason?.message || 'not compared');
    return;
  }
  // A piece: the first save of a new one makes it; later saves update it.
  const v = await ui.command({ cmd: 'import_design', design: toBase64(bytes), ...(s.piece ? { piece: s.piece } : { name: s.title }) });
  if (!v.ok) throw new Error(v.reason?.message || 'not made');
  s.piece = v.data.piece;
  ui.onPiece(s.game, s.piece);
}

// The app writes its files after the command returns: OPFS where it has it, else local storage
// (`t` + text, or `x` + hex).
async function readBack(name, since) {
  for (let i = 0; i < 40; i++) {
    try {
      const dir = await navigator.storage.getDirectory();
      const f = await (await dir.getFileHandle(name)).getFile();
      if (f.lastModified >= since - 1000 && f.size) return new Uint8Array(await f.arrayBuffer());
    } catch { /* not there, or no OPFS */ }
    let v = null;
    try { v = localStorage.getItem(`solidcraft.file.${name}`); } catch { /* blocked */ }
    if (v) {
      if (v[0] === 't') return new TextEncoder().encode(v.slice(1));
      const h = v.slice(1), out = new Uint8Array(h.length / 2);
      for (let j = 0; j < out.length; j++) out[j] = parseInt(h.substr(2 * j, 2), 16);
      return out;
    }
    await sleep(250);
  }
  throw new Error(`SolidCraft didn't save ${name}`);
}

function toBase64(bytes) {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

// A note for the AI agent from an edit's comparison.
export function noteFrom(d) {
  return `Edited in SolidCraft (edit_diff has the details); make the tray do this:\n${d.summary.map((l) => `- ${l}`).join('\n')}`;
}
