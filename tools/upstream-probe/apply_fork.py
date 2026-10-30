"""Apply the 'insert fork' UI patch semantically. Prints which anchors were found."""
import re, sys, pathlib
root = pathlib.Path(sys.argv[1]); U = root / "crates/ui-egui/src"
ok = {}
# 1. GAME BOX tab
p = U / "workspace.rs"; s = p.read_text()
s2, n = re.subn(r'(pub const TABS: &\[&str\] = &\[[^\]]*?)(\];)', r'\1, "GAME BOX"\2', s, count=1)
ok["tab"] = n == 1; s = s2
# 2. GAME BOX section, before the first VIEW section
anchor = '    Section {\n        tab: "VIEW",'
sec = ('    Section {\n        tab: "GAME BOX",\n        panel: "INSERT",\n        promoted: 4,\n'
       '        ids: &["solid.extrude", "solid.shell", "inspect.section", "explode.create"],\n    },\n')
ok["section"] = anchor in s
if ok["section"]: s = s.replace(anchor, sec + anchor, 1)
p.write_text(s)
# 3. module line, sorted
p = U / "lib.rs"; lines = p.read_text().split("\n")
idx = next((i for i, l in enumerate(lines) if re.match(r'pub mod (\w+);', l) and re.match(r'pub mod (\w+);', l).group(1) > "insert_hud"), None)
ok["mod"] = idx is not None
if idx is not None: lines.insert(idx, "pub mod insert_hud;")
p.write_text("\n".join(lines))
# 4. viewport overlay hook
p = U / "viewport.rs"; s = p.read_text()
a = "fn overlays(app: &SolveApp, painter: &egui::Painter, proj: &Proj) {\n"
ok["hook"] = a in s
if ok["hook"]: s = s.replace(a, a + "    crate::insert_hud::draw(app, painter, proj);\n", 1)
p.write_text(s)
(U / "insert_hud.rs").write_text("//! Insert fit HUD (fork).\nuse crate::SolveApp;\n\npub fn draw(_app: &SolveApp, _painter: &egui::Painter, _proj: &crate::viewport::Proj) {}\n")
print(" ".join(f"{k}={'ok' if v else 'LOST'}" for k, v in ok.items()))
