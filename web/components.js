// The Components tab: the project's component list, and the form that records each one as
// measured (piece or stack, extra measurements, photos with notes, how it should go in, what it
// goes with). How it's stored is optional here; `store_as` on the server turns the measurements into
// the packer's fields, or says what's missing.

const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const mm = (v) => `${(+v).toFixed(2).replace(/\.?0+$/, '')}`;

export function slugify(name) {
  return String(name).toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 48).replace(/-+$/, '');
}

const STORE = [
  ['undecided', 'Decide later', 'No pocket yet. You or the AI agent pick from the measurements and notes.'],
  ['cards', 'Cards', 'Card size from the footprint; stack height from the measured stack (scaled to the count) or the count.'],
  ['tokens', 'Tokens', 'Stacked in pockets: footprint, one token’s thickness (or a stack ÷ its count) and the count.'],
  ['mini', 'Mini', 'One round pocket each: the wider side of one mini as the base, its height, and the count.'],
  ['bin', 'Loose bin', 'An open bin the size of the measured pile.'],
  ['as_is', 'As-is, no insert', 'Goes in the box as it is, like a board or the rulebook. It lies in the top stack.'],
  ['box', 'Box', 'A box holding an insert: the game’s own (a base with a telescoping lid: measure its inside, and optionally its outside closed; photos of the lid dress the 3D view), or one to print, sized to what goes in it, with a slide-on lid.'],
];
const SHAPES = [['rect', 'Rectangle'], ['square', 'Square'], ['round', 'Round'], ['hex', 'Hex'], ['irregular', 'Irregular']];
const STATUS = { box: ['box', ''], placed: ['in the insert', 'good'], as_is: ['as-is', ''], undecided: ['not decided', 'warn'], unplaced: ['no place yet', 'bad'], no_box: ['no box yet', 'warn'] };

let api = null;
// The component in the form: an id, 'new', or null.
let editing = null;
// The editor is only redrawn when what it shows changes, so live updates don't eat typing.
let editorKey = null;

// A kind to preselect on the next new component (the Insert tab's "Add a box").
let newKind = null;

export function initComponents(a) {
  api = a;
  $('comp-add').addEventListener('click', () => { newKind = null; edit('new'); });
  $('box-add-comp').addEventListener('click', () => { newKind = 'box'; edit('new'); });
}

export function editComponent(id) {
  newKind = null;
  edit(id);
}

export function newComponent(kind) {
  newKind = kind;
  edit('new');
}

const dimsText = (d) => d.map(mm).join(' × ') + ' mm';

/// "One piece 63.5 × 88 × 0.3 mm · stack of 40: 63.5 × 88 × 12 mm · 120 in the game"
export function describeMeasure(c) {
  const m = c.measure;
  if (!m) return '';
  const parts = [];
  const piece = m.piece && `one piece ${dimsText(m.piece)}`;
  const stack = m.stack && `${m.stack_count ? `stack of ${m.stack_count}` : 'pile'} ${dimsText(m.stack)}`;
  if (m.of === 'stack') parts.push(stack, piece); else parts.push(piece, stack);
  if (m.count) parts.push(`${m.count} in the game`);
  if (m.shape) parts.push(m.shape);
  return parts.filter(Boolean).join(' · ');
}

/// What the packer was given.
function describeStorage(c) {
  if (c.kind === 'cards') return `cards ${c.size.join(' × ')} mm · ${c.stack ? `stack ${c.stack} mm` : `${c.count} cards${c.sleeve ? `, ${c.sleeve} sleeves` : ''}`}`;
  if (c.kind === 'mini') return `mini, base ⌀${c.base} · ${c.height} mm tall · ×${c.count}`;
  if (c.kind === 'tokens') return `tokens, ${c.shape} ${c.size.join(' × ')} mm · ${c.thickness} mm · ×${c.count}`;
  if (c.kind === 'bin') return `bin ${c.size.join(' × ')} mm · ${c.depth} mm deep`;
  if (c.kind === 'as_is') return 'goes in as it is';
  if (c.kind === 'box' && c.printed) {
    const sh = api.state()?.boxes?.find((b) => b.id === c.id)?.shell;
    return `box to print, sized to fit${sh ? `: ${sh.outer.map(mm).join(' × ')} mm outside` : ''} · slide-on lid${c.top_stack ? ` · top stack ${mm(c.top_stack)} mm` : ''}`;
  }
  if (c.kind === 'box') return `box, inside ${c.inner.map(mm).join(' × ')} mm · lid ${c.lid_height ? `${mm(c.lid_height)} mm deep` : 'full depth'}${c.lid_depth ? `, ${mm(c.lid_depth)} mm room above the rim` : ''}${c.lid_lift ? `, lift up to ${mm(c.lid_lift)} mm` : ''} · top stack ${mm(c.top_stack || 0)} mm`;
  return 'not decided';
}

function photoUrl(file) {
  return `/api/games/${encodeURIComponent(api.game())}/media/photos/${encodeURIComponent(file)}`;
}

export function renderComponents() {
  const st = api.state();
  if (!st?.spec) return;
  const s = st.spec;
  const place = Object.fromEntries((st.placement || []).map((p) => [p.id, p]));
  if (editing && editing !== 'new' && !s.components.some((c) => c.id === editing)) editing = null;
  if (!editing && !s.components.length) editing = 'new';
  const row = (c) => {
    const p = place[c.id];
    const [label, cls] = STATUS[p?.status] || ['', ''];
    const photo = c.photos?.[0];
    return `<button class="comp-row ${editing === c.id ? 'sel' : ''}" data-id="${esc(c.id)}">
      <span class="comp-thumb">${photo ? `<img src="${photoUrl(photo.file)}" alt="">` : ''}</span>
      <span class="comp-main"><b>${esc(c.name)}${c.printed ? ' <span class="badge print">PRINT</span>' : ''}</b>
        <span class="d">${esc(c.kind === 'box' ? describeStorage(c) : describeMeasure(c) || describeStorage(c))}</span>
        <span class="d"><span class="${cls}">${esc(c.kind === 'box' ? p?.detail : label)}</span>${p?.box && (st.boxes || []).length > 1 ? ` · ${esc(st.boxes.find((b) => b.id === p.box)?.name || p.box)}` : ''}${c.notes ? ` · ${esc(c.notes)}` : ''}</span></span>
    </button>`;
  };
  // Boxes first, apart: everything else goes in one of them.
  $('box-list').innerHTML = s.components.filter((c) => c.kind === 'box').map(row).join('')
    || '<p class="muted small">No box yet. Measure the game’s box, or have one designed to print, sized to what goes in it.</p>';
  $('comp-list').innerHTML = s.components.filter((c) => c.kind !== 'box').map(row).join('') || '<p class="muted small">Nothing yet. Measure a piece and add it.</p>';
  for (const b of document.querySelectorAll('#box-list [data-id], #comp-list [data-id]')) b.addEventListener('click', () => edit(b.dataset.id));
  const c = editing === 'new' ? null : s.components.find((x) => x.id === editing);
  const key = editing === 'new' ? 'new' : c ? JSON.stringify(c) + JSON.stringify(place[c.id]) + (st.boxes || []).length + JSON.stringify(st.textures?.[c.id] || {}) : 'none';
  if (key !== editorKey) {
    editorKey = key;
    renderEditor(c);
  }
}

function edit(id) {
  editing = id;
  editorKey = null;
  renderComponents();
  if (matchMedia('(max-width: 900px)').matches) $('comp-editor').scrollIntoView({ behavior: 'smooth', block: 'start' });
}

function num(v) {
  return v === undefined || v === null ? '' : v;
}

function dimsInputs(prefix, d) {
  d = d || [];
  return `<div class="dims-row">
    <label>Width <input name="${prefix}0" type="number" step="0.01" min="0" inputmode="decimal" value="${num(d[0])}"></label>
    <label>Depth <input name="${prefix}1" type="number" step="0.01" min="0" inputmode="decimal" value="${num(d[1])}"></label>
    <label>Height <input name="${prefix}2" type="number" step="0.01" min="0" inputmode="decimal" value="${num(d[2])}"></label></div>`;
}

function renderEditor(c) {
  const st = api.state();
  const s = st.spec;
  const m = c?.measure || { of: 'piece' };
  const of = m.of || 'piece';
  const other = of === 'piece' ? 'stack' : 'piece';
  const hasOther = !!m[other];
  const others = s.components.filter((x) => x.id !== c?.id && x.kind !== 'box');
  const p = c && (st.placement || []).find((x) => x.id === c.id);
  const kind = c?.kind || newKind || 'undecided';
  newKind = null;
  const bk = c?.kind === 'box' ? { inner: [], ...c } : { inner: [], lid_depth: 0, top_stack: 0 };
  const made = bk.printed ? 'printed' : 'bought';
  const maxIn = bk.printed?.max_inner || [];
  const shell = c && st.boxes?.find((b) => b.id === c.id)?.shell;
  const boxes = st.boxes || [];
  $('comp-editor').innerHTML = `
  <form id="comp-form" class="form card" novalidate>
    <div class="sheet-head"><h2>${c ? `Edit ${esc(c.name)}` : 'New component'}</h2>
      ${c ? `<code class="muted small">${esc(c.id)}</code>` : ''}</div>
    <label>Name <input name="name" required value="${esc(c?.name || '')}" placeholder="Event cards, coins, player boards…"></label>

    <fieldset class="for-box"><legend>The box</legend>
      <div class="seg" role="radiogroup">
        <label><input type="radio" name="made" value="bought" ${made === 'bought' ? 'checked' : ''}> The game’s box — measure it</label>
        <label><input type="radio" name="made" value="printed" ${made === 'printed' ? 'checked' : ''}> Print one — sized to fit</label>
      </div>
      <div class="for-printed">
        <p class="small">Sized to what goes in it: the smallest box that holds its trays, with 45° dovetail grooves for a lid that slides in from one end and clicks shut. Nothing needs supports.</p>
        ${shell ? `<p class="small">Now: <b>${shell.outer.map(mm).join(' × ')} mm</b> outside, ${shell.inner.map(mm).join(' × ')} mm inside; lid ${shell.lid.slice(0, 2).map(mm).join(' × ')} × ${mm(shell.lid[2])} mm. <span class="muted">${esc(shell.why)}</span></p>` : c ? '<p class="small muted">Sized once something goes in it.</p>' : ''}
        <p class="small muted">Largest inside allowed, mm <span class="muted">— optional, to fit a shelf or another box; empty means what the bed prints</span></p>
        <div class="dims-row">
          <label>Length <input name="XL" type="number" step="0.1" min="0" inputmode="decimal" value="${maxIn[0] || ''}"></label>
          <label>Width <input name="XW" type="number" step="0.1" min="0" inputmode="decimal" value="${maxIn[1] || ''}"></label>
          <label>Height <input name="XH" type="number" step="0.1" min="0" inputmode="decimal" value="${maxIn[2] || ''}"></label></div>
      </div>
      <p class="small muted for-bought">Inside of the base, mm <span class="muted">— a base with a telescoping lid</span></p>
      <div class="dims-row for-bought">
        <label>Length <input name="BL" type="number" step="0.1" min="0" inputmode="decimal" value="${num(bk.inner[0])}"></label>
        <label>Width <input name="BW" type="number" step="0.1" min="0" inputmode="decimal" value="${num(bk.inner[1])}"></label>
        <label>Height <input name="BH" type="number" step="0.1" min="0" inputmode="decimal" value="${num(bk.inner[2])}"></label></div>
      <div class="two">
        <label>Top stack <span class="muted small">— boards and rules on top of the insert</span>
          <input name="top_stack" type="number" step="0.1" min="0" inputmode="decimal" value="${num(bk.top_stack)}"></label>
        <label class="for-bought">Lid lift allowed <span class="muted small">— how high the closed lid may ride when the insert stands above the rim; empty for none. Used only if needed</span>
          <input name="lid_lift" type="number" step="0.1" min="0" inputmode="decimal" value="${bk.lid_lift ? num(bk.lid_lift) : ''}"></label>
      </div>
      <div class="two for-bought">
        <label>Lid height <span class="muted small">— outside height of the lid’s sides; empty when it telescopes the full depth</span>
          <input name="lid_height" type="number" step="0.1" min="0" inputmode="decimal" value="${num(bk.lid_height)}"></label>
        <label>Room above the rim <span class="muted small">— 0 when the closed lid rests on the rim, as usual</span>
          <input name="lid_depth" type="number" step="0.1" min="0" inputmode="decimal" value="${num(bk.lid_depth)}"></label>
      </div>
    </fieldset>

    <fieldset class="for-bought"><legend><span class="not-box">What did you measure?</span><span class="for-box">The outside of the closed box <span class="muted small">— optional, for the 3D view</span></span></legend>
      <div class="seg not-box" role="radiogroup">
        <label><input type="radio" name="of" value="piece" ${of === 'piece' ? 'checked' : ''}> One piece</label>
        <label><input type="radio" name="of" value="stack" ${of === 'stack' ? 'checked' : ''}> A stack or pile</label>
      </div>
      <div id="main-dims"></div>
      <details id="other-dims" class="not-box" ${hasOther ? 'open' : ''}><summary></summary><div id="other-body"></div></details>
    </fieldset>

    <div class="two not-box">
      <label>How many in the game <input name="count" type="number" min="1" step="1" inputmode="numeric" value="${num(m.count)}"></label>
      <label>Shape from above <select name="shape"><option value="">—</option>${SHAPES.map(([v, t]) => `<option value="${v}" ${m.shape === v ? 'selected' : ''}>${t}</option>`).join('')}</select></label>
    </div>

    <fieldset><legend>More measurements <span class="muted small">— a thinner or wider part, the sides of a hex…</span></legend>
      <div id="extras"></div>
      <button type="button" class="toggle" id="extra-add">Add a measurement</button>
    </fieldset>

    <label>How should it go in? <span class="muted small">— a pile, stacks of 20, sorted by player…</span>
      <textarea name="notes" rows="2">${esc(c?.notes || '')}</textarea></label>

    <fieldset class="not-box"><legend>Used with <span class="muted small">— kept near these at the table</span></legend>
      <div class="chips">${others.map((o) => `<label class="chip"><input type="checkbox" name="goes_with" value="${esc(o.id)}" ${(c?.goes_with || []).includes(o.id) ? 'checked' : ''}> ${esc(o.name)}</label>`).join('') || '<span class="muted small">No other components yet.</span>'}</div>
    </fieldset>

    <fieldset><legend>Stored as <span class="muted small">— optional</span></legend>
      <div class="chips">${STORE.map(([v, t]) => `<label class="chip"><input type="radio" name="kind" value="${v}" ${kind === v ? 'checked' : ''}> ${t}</label>`).join('')}</div>
      <p class="muted small" id="kind-help"></p>
      ${c && kind !== 'undecided' ? `<p class="small">Packer has: ${esc(describeStorage(c))}</p>` : ''}
      ${p ? `<p class="small">Where: <span class="${(STATUS[p.status] || [])[1] || ''}">${esc(p.detail)}</span></p>` : ''}
    </fieldset>

    <div class="two not-box">
      ${boxes.length > 1 ? `<label>Box <select name="box">${boxes.map((b, i) => `<option value="${esc(b.id)}" ${(c?.box || boxes[0].id) === b.id ? 'selected' : ''}>${esc(b.name)}${i === 0 ? ' (first)' : ''}</option>`).join('')}</select></label>` : ''}
      ${(s.groups || []).length ? `<label>Group <select name="group"><option value="">—</option>${s.groups.map((g) => `<option value="${esc(g.id)}" ${c?.group === g.id ? 'selected' : ''}>${esc(g.name)}</option>`).join('')}</select></label>` : ''}
    </div>

    ${c ? `<fieldset><legend>Photos</legend>
      <div class="photo-list">${(c.photos || []).map((ph) => `<div class="photo-item">
        <img src="${photoUrl(ph.file)}" alt="">
        <input data-photo-note="${esc(ph.file)}" value="${esc(ph.note || '')}" placeholder="What it shows">
      </div>`).join('') || '<span class="muted small">No photos yet.</span>'}</div>
      <div class="toggles">
        <label class="toggle file-btn">Add photo<input id="comp-photo" type="file" accept="image/*" hidden></label>
        ${(c.photos || []).length ? `<button type="button" class="toggle" id="comp-faces">${c.kind === 'box' ? 'Mark the lid’s top, front and side' : 'Mark faces for the 3D view'}</button>` : ''}
      </div></fieldset>` : '<p class="muted small">Photos can be added once it is saved.</p>'}

    <p id="comp-err" class="bad small" hidden></p>
    <div class="form-actions">
      ${c ? '<button type="button" class="toggle danger" id="comp-del">Delete</button>' : ''}
      ${c ? `<button type="button" class="toggle" id="comp-view">View in 3D</button>` : ''}
      <span class="spacer"></span>
      <button class="primary" type="submit">${c ? 'Save' : 'Add component'}</button>
    </div>
  </form>`;

  const f = $('comp-form');
  // Main and optional other measurement, swapped when "what did you measure" changes.
  const stackCount = (v) => `<label>Pieces in the stack <span class="muted small">— leave empty for a loose pile</span>
      <input name="SC" type="number" min="1" step="1" inputmode="numeric" value="${num(v)}"></label>`;
  let vals = { piece: m.piece, stack: m.stack, sc: m.stack_count };
  const drawDims = () => {
    const o = f.of.value;
    const ot = o === 'piece' ? 'stack' : 'piece';
    $('main-dims').innerHTML = `<p class="small muted not-box">${o === 'piece' ? 'One piece' : 'The stack or pile'}, mm</p>${dimsInputs('M', vals[o])}${o === 'stack' ? stackCount(vals.sc) : ''}`;
    $('other-dims').querySelector('summary').textContent = ot === 'piece' ? 'Also measure one piece (optional)' : 'Also measure a stack or pile (optional)';
    $('other-body').innerHTML = `${dimsInputs('O', vals[ot])}${ot === 'stack' ? stackCount(vals.sc) : ''}`;
  };
  const readDims = (prefix) => [0, 1, 2].map((i) => f[`${prefix}${i}`].value.trim());
  drawDims();
  for (const r of f.querySelectorAll('[name=of]')) {
    r.addEventListener('click', () => {
      // The radio has already flipped: what was typed belongs to the previous choice.
      const now = f.of.value, before = now === 'piece' ? 'stack' : 'piece';
      vals[before] = readDims('M').map((v) => v === '' ? undefined : +v);
      vals[now] = readDims('O').map((v) => v === '' ? undefined : +v);
      if (f.SC) vals.sc = f.SC.value === '' ? undefined : +f.SC.value;
      drawDims();
    });
  }

  // Extra measurement rows.
  const extras = (m.extra || []).map((e) => ({ ...e }));
  const drawExtras = () => {
    $('extras').innerHTML = extras.map((e, i) => `<div class="extra-row">
      <input data-x="label" data-i="${i}" value="${esc(e.label)}" placeholder="What (e.g. hex side)">
      <input data-x="mm" data-i="${i}" type="number" step="0.01" min="0" inputmode="decimal" value="${num(e.mm)}" placeholder="mm">
      <button type="button" class="toggle" data-rm="${i}" aria-label="Remove">✕</button></div>`).join('');
    for (const x of $('extras').querySelectorAll('[data-x]')) x.addEventListener('input', () => { extras[+x.dataset.i][x.dataset.x] = x.value; });
    for (const b of $('extras').querySelectorAll('[data-rm]')) b.addEventListener('click', () => { extras.splice(+b.dataset.rm, 1); drawExtras(); });
  };
  drawExtras();
  $('extra-add').addEventListener('click', () => { extras.push({ label: '', mm: '' }); drawExtras(); $('extras').querySelector('.extra-row:last-child input')?.focus(); });

  const help = () => {
    $('kind-help').textContent = STORE.find(([v]) => v === f.kind.value)?.[2] || '';
    f.classList.toggle('is-box', f.kind.value === 'box');
    // Only a box is printed; everything else shows its measurements.
    f.classList.toggle('is-printed', f.kind.value === 'box' && f.made.value === 'printed');
  };
  for (const r of f.querySelectorAll('[name=kind], [name=made]')) r.addEventListener('change', help);
  help();

  const err = (msg) => { $('comp-err').hidden = !msg; $('comp-err').textContent = msg || ''; };

  f.addEventListener('submit', async (e) => {
    e.preventDefault();
    err(null);
    const name = f.name.value.trim();
    if (!name) return err('Give it a name.');
    const o = f.of.value, ot = o === 'piece' ? 'stack' : 'piece';
    const main = readDims('M'), oth = readDims('O');
    const full = (d) => d.every((v) => v !== '' && +v > 0);
    const empty = (d) => d.every((v) => v === '');
    const isBox = f.kind.value === 'box';
    const printed = isBox && f.made.value === 'printed';
    const inner = [f.BL.value, f.BW.value, f.BH.value];
    if (isBox && !printed && !full(inner)) return err('Measure the inside of the box: length, width and height, in mm.');
    const lim = [f.XL.value, f.XW.value, f.XH.value].map((v) => +v || 0);
    if (lim.some((v) => v < 0)) return err('The largest inside is in mm, or empty.');
    // A printed box has no measured inside or lid; the packer sizes it.
    const boxFields = !isBox ? {}
      : printed ? { printed: lim.some((v) => v > 0) ? { max_inner: lim } : {}, top_stack: +f.top_stack.value || 0, inner: null, lid_depth: 0, lid_height: null, lid_lift: 0 }
      : { inner: inner.map(Number), lid_depth: +f.lid_depth.value || 0, top_stack: +f.top_stack.value || 0, lid_height: +f.lid_height.value || null, lid_lift: +f.lid_lift.value || 0, printed: null };
    let measure = null;
    if (printed) measure = c?.measure || null;
    else if (!empty(main) || (!c && !isBox) || c?.measure) {
      if (!full(main)) return err(`Measure the ${o === 'piece' ? 'piece' : 'stack'}: width, depth and height, in mm.`);
      if (!empty(oth) && !full(oth)) return err(`The optional ${ot}: give all three measurements, or none.`);
      measure = { of: o, [o]: main.map(Number) };
      if (full(oth)) measure[ot] = oth.map(Number);
      const sc = f.SC?.value;
      if (sc && (o === 'stack' || full(oth))) measure.stack_count = +sc;
      if (f.count.value) measure.count = +f.count.value;
      if (f.shape.value) measure.shape = f.shape.value;
      const ex = extras.filter((x) => String(x.label).trim() || x.mm !== '');
      if (ex.some((x) => !String(x.label).trim() || !(+x.mm > 0))) return err('Each extra measurement needs a description and a size in mm.');
      if (ex.length) measure.extra = ex.map((x) => ({ label: String(x.label).trim(), mm: +x.mm }));
    }
    const goes = isBox ? [] : [...f.querySelectorAll('[name=goes_with]:checked')].map((x) => x.value);
    const notes = f.notes.value.trim();
    const box = isBox ? undefined : f.box ? (f.box.value === boxes[0]?.id && !c?.box ? null : f.box.value) : undefined;
    const group = isBox ? undefined : f.group ? (f.group.value || null) : undefined;
    const kindNow = f.kind.value;
    const measureChanged = JSON.stringify(measure) !== JSON.stringify(c?.measure || null);
    let id = c?.id;
    let v;
    if (!c) {
      id = slugify(name) || 'component';
      const ids = new Set([...s.components.map((x) => x.id), ...(s.groups || []).map((g) => g.id)]);
      for (let i = 2; ids.has(id); i++) id = `${slugify(name)}-${i}`;
      const body = { cmd: 'add_component', id, name, kind: isBox ? 'box' : 'undecided', ...Object.fromEntries(Object.entries(boxFields).filter(([, v]) => v !== null)) };
      if (measure) body.measure = measure;
      if (notes) body.notes = notes;
      if (goes.length) body.goes_with = goes;
      if (box) body.box = box;
      if (group) body.group = group;
      v = await api.command(body);
    } else {
      const body = { cmd: 'update_component', id, name, notes: notes || null, goes_with: goes.length ? goes : null };
      // A box stays a box: its inside changes in place, keeping its layout.
      if (isBox && c.kind === 'box') Object.assign(body, boxFields);
      if (measure || c.measure) body.measure = measure;
      if (box !== undefined) body.box = box;
      if (group !== undefined) body.group = group;
      v = await api.command(body);
    }
    if (!v.ok) return err(v.reason?.message);
    // Re-derive the packer's fields when the choice or the measurements changed.
    if (c && kindNow !== c.kind || !c && !isBox && kindNow !== 'undecided' || (c && measureChanged && !['undecided', 'as_is', 'box'].includes(kindNow))) {
      const r = await api.command({ cmd: 'store_as', id, kind: kindNow, ...Object.fromEntries(Object.entries(boxFields).filter(([, v]) => v !== null)) });
      if (!r.ok) {
        editing = id; editorKey = null;
        await api.refresh();
        return err(`Saved, but not stored as ${kindNow} yet: ${r.reason?.message}`);
      }
    }
    api.toast(c ? `Saved ${name}` : `Added ${name}`);
    // A new one stays open so photos can be added next; "Add component" starts the next one.
    editing = id;
    editorKey = null;
    await api.refresh();
  });

  if (c) {
    $('comp-del').addEventListener('click', async () => {
      if (!confirm(`Delete ${c.name}?`)) return;
      const v = await api.command({ cmd: 'remove_component', id: c.id });
      if (v.ok) { editing = null; editorKey = null; await api.refresh(); }
    });
    $('comp-view')?.addEventListener('click', () => {
      const placed = p?.status === 'placed' || p?.status === 'as_is';
      api.view(c.id, placed ? 'box' : 'alone');
    });
    for (const x of f.querySelectorAll('[data-photo-note]')) {
      x.addEventListener('change', () => api.command({ cmd: 'set_photo_note', component: c.id, photo: x.dataset.photoNote, note: x.value.trim() || null }));
    }
    $('comp-faces')?.addEventListener('click', () => api.openSheet(c.id));
    $('comp-photo').addEventListener('change', async (e) => {
      const file = e.target.files[0];
      e.target.value = '';
      if (!file) return;
      api.toast('Uploading photo…');
      // Shrink on the device: a camera photo is several MB, 2560 px is plenty.
      const bmp = await createImageBitmap(file);
      const k = Math.min(1, 2560 / Math.max(bmp.width, bmp.height));
      const cv = document.createElement('canvas');
      cv.width = Math.round(bmp.width * k); cv.height = Math.round(bmp.height * k);
      cv.getContext('2d').drawImage(bmp, 0, 0, cv.width, cv.height);
      const note = prompt('What does this photo show? (optional)') || null;
      const v = await api.command({ cmd: 'add_photo', component: c.id, data: cv.toDataURL('image/jpeg', 0.88), note });
      if (v.ok) { api.toast('Photo added'); editorKey = null; await api.refresh(); }
    });
  }
}
