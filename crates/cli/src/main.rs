//! `bid`: a thin command-line client for the insert server's command API.
//!
//!     bid [-g GAME] [--json] <command> [key=value | key:=json]...
//!
//! It sends `{ "cmd": <command>, "game": GAME, key: value, ... }` to `POST /api/command`, the
//! same endpoint the web UI uses. Values are inferred: `66x91` → [66, 91], `1,2,3` → [1, 2, 3],
//! numbers, true/false/null, and JSON when it starts with `[` or `{`; anything else is a string.
//! `key:=<json>` passes raw JSON (e.g. `name:='"120"'` to force a string).
//!
//! `bid mcp` runs a stdio MCP server whose tools are the same commands, forwarded to the same
//! endpoint, so Claude can drive the designer without a shell.

use serde_json::{Map, Value, json};

const HELP: &str = "\
bid — drive BoxInsertDesigner (server: $BID_URL, else [server] bind in the
games folder's bid.toml, else http://127.0.0.1:8102)

usage: bid [-g GAME] [--json] <command> [key=value | key:=json]...

commands (all take an optional game; default is the current game):
  list_games | use_game -g SLUG | get_state | generate_layouts [box=..]
  create_game [-g SLUG] name=... [inner=LxWxH lid_depth=.. top_stack=..]   (slug defaults from the name)
  set_game [name=..] [notes=..]
  add_box id=.. name=.. inner=LxWxH [lid_depth=..] [top_stack=..] [source=..]  |  remove_box id=..
  add_box id=.. name=.. printed:=true [max_inner=LxWxH] [top_stack=..]   (a box to print, sized to what goes in it)
  set_box [box=..] [inner=LxWxH] [lid_depth=..] [lid_height=..] [lid_lift=..] [top_stack=..] [source=measured|estimate] [name=..]
          [printed:=true|false] [max_inner=LxWxH]   (a printed box: slide-on lid; 0 in max_inner leaves that side to the bed)
  (commands about one box take box=<id>; the default is the first box)

  add_component id=.. name=.. kind=undecided measure:='{\"of\":\"piece\",\"piece\":[W,D,H],\"count\":N}' [notes=..] [goes_with:='[..]'] [box=..]
     measure: of=piece|stack, piece [W,D,H], stack [W,D,H], stack_count, count, shape=rect|square|round|hex|irregular,
              extra [{label, mm}]
  store_as id=.. kind=cards|tokens|mini|bin|as_is|undecided|box [field=value...]   (fields come from measure; given ones win)
  (a box is a component: kind=box inner=LxWxH [lid_depth=..] [top_stack=..] [lid_height=..] [lid_lift=..]; its measure is
   the outside, closed; the lid telescopes: lid_height is how far down (unset: full depth); lid_lift is how far
   the closed lid may ride above the rim, used only when the insert needs it;
   faces lid (its top), front (a long side of the lid), side (a short side of the lid))
  add_component id=.. name=.. kind=cards size=WxL (count=N [sleeve=premium] | stack=MM) [group=..]
  add_component id=.. name=.. kind=mini base=D height=H [count=N] [group=..]
  add_component id=.. name=.. kind=tokens shape=round|square|rect|hex size=D|WxH thickness=T count=N
  add_component id=.. name=.. kind=bin size=LxW depth=D
  (any component: clearance=MM per side, if it needs more than the style's 0.8)
  update_component id=.. <field>=<value>...   (field=null removes it)
  remove_component id=..
  add_photo component=.. file=photo.jpg [note=..]  |  set_photo_note component=.. photo=<file> note=..
  set_face component=.. face=top|front photo=<file> corners:='[[x,y],[x,y],[x,y],[x,y]]'   (0-1, TL TR BR BL)
  remove_photo component=.. photo=<file>
  add_group id=.. name=..  |  remove_group id=..
  add_sleeve id=.. name=.. per_card_mm=..
  set_style [walls.normal=2.0] [tolerance.print_to_component=0.8] [floor.min=..] ...   (no args: show it)
  choose_layout [box=..] strategy=by-group|by-kind|fewest-trays
  tweak_tray [box=..] tray=.. [max_depth=MM] [walls=MM] [finger_holes=bool] [open_ends=bool] [per_pocket=bool] [holders=bool] [grips=bool] [half_circle=middle|<component>] [scoops=bool] [color=#rrggbb]
             [label=TEXT] [pocket_labels=bool] [label_mode=inlay|cut]   (stencil labels: on its top, on each pocket's floor; placed and sized by the packer)
             [order:='[\"comp-a\",\"comp-b\"]']   (pockets along the tray in this order; [] hands it back)
             [pocket=<component> [scoops=bool] [notch=MM] [notch_keep=MM] [notch_floor=MM] [fill_spare=bool]]   (one pocket: its scoop; a slot through its long walls, keeping MM above its floor, or with keep 0 biting MM into the floor; the leftover room instead of a spare)
             [name=..] [notes=..] [unset:='[\"walls\"]'] [clear=true]   (empty name or notes clears it; notes are changes the user wants, for Claude)
  plates   (every piece to print on the X1C's plates: where, turned how, and any problem)
  set_plate piece=<box>/<tray> [plate=N] [rotate=0|90|180|270] [flip=bool] [unset:='[\"plate\"]'] [clear=true]   (piece=* clear=true: all back to the packer)
  set_plate piece=.. [filament=N] [parts:='{\"Label\": 2}']   (a piece's filament slot, and its named parts')
  name_plate plate=N name=..  |  set_filament slot=N [preset=..] [color=#rrggbb]  |  remove_filament slot=N
  set_labels [mode=inlay|cut] [filament=N]   (the game's labels: inlay = a flush part in slot N's filament; cut = just cut)
  import_project file=project.3mf [replace=true]   (a Bambu Studio project's objects as pieces, as they are)
  set_copies piece=.. copies:='[{\"name\":\"Harmony\",\"filament\":6}]'
  place_piece piece=.. [copy=..] at:='[x,y,z]' [turn=0|90|180|270] [tilt=..] [flip=bool] [inside=<piece>[/<copy>]] [layer=N] [unplace=true]
  remove_piece piece=..
  export   (STLs, the insert 3MF, and <slug>-plates.3mf: a Bambu Studio project of the plates)

  requests [status=pending|open|working|done|declined|all]   (every game unless -g; tray notes left for an agent)
  wait_requests [since_ms=..] [timeout_s=50]   (blocks until an open request is new or changed; returns now_ms)
  update_request -g GAME id=.. status=working|done|declined|open [reply=..]
  agent_status   (an agent watching the queue from outside, and the app's own)
  set_agent [api_key=..] [remove_key=true] [model=..] [effort=low|medium|high|xhigh|max] [enabled=bool]   (the app's own agent; the key is checked, kept privately, never shown)
  mark_read -g GAME id=..   (the user has read the answer: off the tray, into its history)
  log_ask tray=.. text=.. reply=.. [box=..]   (a change asked for directly, not as a tray note, once done)
  tray_history tray=.. [box=..]   (a tray's requests, direct asks and the saved changes naming it)
  edit_diff tray=..   (the user's last hand edit of the tray in SolidCraft, against the generated tray)
  edit_script tray=.. [box=..]   (the tray's SolidCraft script, as the app's Edit in SolidCraft loads it)
  keep_edit tray=.. [box=..]  |  drop_edit tray=.. [box=..]   (print the tray as its last SolidCraft edit, or as generated again)
  import_design design=<base64 .sc> (name=.. | piece=..)  |  piece_design piece=..   (pieces made in SolidCraft)

  history [limit=N]  |  save_version name=.. [note=..]  |  restore_version rev=<commit|tag>
     (games/ is its own git repo; the server commits each game ~30 s after changes settle)

  mcp        run as a stdio MCP server exposing these commands as tools
";

// (tool name, description) for the MCP server. Arguments are the command's own fields.
include!("../../server/src/tools_table.rs");

/// Who is calling, for the app's agent status: `bid` or `bid mcp` (and the MCP client), the
/// machine and folder, and the Claude Code session when it runs in one.
static CLIENT: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);

fn client_info(via: &str) -> Value {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    // Windows names the machine in COMPUTERNAME, and its `hostname` takes no -s.
    let host = env("COMPUTERNAME").or_else(|| {
        std::process::Command::new("hostname").arg("-s").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    });
    json!({
        "via": via, "host": host, "pid": std::process::id(),
        "cwd": std::env::current_dir().ok().map(|d| d.display().to_string()),
        "claude_session": env("CLAUDE_CODE_SESSION_ID"), "remote_session": env("CLAUDE_CODE_BRIDGE_SESSION_ID"),
        "entrypoint": env("CLAUDE_CODE_ENTRYPOINT"),
    })
}

/// `$BID_URL`, else `http://` + `[server] bind` from `bid.toml` in the games folder (`$BID_GAMES`,
/// else `games/` in the checkout this binary was built in, else in the current folder).
fn server_url() -> String {
    if let Some(u) = std::env::var("BID_URL").ok().filter(|v| !v.is_empty()) {
        return u;
    }
    let checkout = std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()).and_then(|e| e.ancestors().nth(3).map(|p| p.join("games")));
    let dirs = std::env::var("BID_GAMES").ok().filter(|v| !v.is_empty()).map(std::path::PathBuf::from).into_iter().chain(checkout).chain([std::path::PathBuf::from("games")]);
    for dir in dirs {
        let Ok(text) = std::fs::read_to_string(dir.join("bid.toml")) else { continue };
        let mut section = "";
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                section = line;
            } else if section == "[server]" && line.starts_with("bind") {
                if let Some(v) = line.split('"').nth(1) {
                    return format!("http://{v}");
                }
            }
        }
        break;
    }
    "http://127.0.0.1:8102".into()
}

fn post(url: &str, body: Value) -> Result<Value, String> {
    // A header is ASCII: anything else (a folder's name) goes as a JSON escape.
    let who: String = CLIENT.lock().unwrap().get_or_insert_with(|| client_info("bid")).to_string().encode_utf16().map(|u| if u < 0x80 { (u as u8 as char).to_string() } else { format!("\\u{u:04x}") }).collect();
    let resp = match ureq::post(&format!("{url}/api/command")).set("x-bid-client", &who).send_json(body) {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => return Err(format!("can't reach {url}: {e}")),
    };
    resp.into_json().map_err(|e| format!("bad response: {e}"))
}

fn mcp(url: &str) {
    use std::io::{BufRead, Write};
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue }; // notifications need no reply
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => {
                let mut who = client_info("bid mcp");
                let ci = &msg["params"]["clientInfo"];
                if let Some(n) = ci["name"].as_str() {
                    who["mcp_client"] = json!(format!("{n} {}", ci["version"].as_str().unwrap_or("")).trim().to_string());
                }
                *CLIENT.lock().unwrap() = Some(who);
                json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "box-insert-designer", "version": env!("CARGO_PKG_VERSION") },
            })
            }
            "ping" => json!({}),
            "tools/list" => json!({ "tools": TOOLS.iter().map(|(n, d)| json!({
                "name": n, "description": d,
                "inputSchema": { "type": "object", "properties": { "game": { "type": "string", "description": "game slug; default: the current game" } }, "additionalProperties": true },
            })).collect::<Vec<_>>() }),
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or("");
                let mut args = msg["params"]["arguments"].as_object().cloned().unwrap_or_default();
                let (text, err) = if name == "help" {
                    (HELP.to_string(), false)
                } else {
                    args.insert("cmd".into(), json!(name));
                    match post(url, Value::Object(args)) {
                        Ok(v) => (serde_json::to_string_pretty(&v).unwrap(), v["ok"].as_bool() != Some(true)),
                        Err(e) => (e, true),
                    }
                };
                json!({ "content": [{ "type": "text", "text": text }], "isError": err })
            }
            m => {
                let e = json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("no method {m}") } });
                let _ = writeln!(out, "{e}");
                let _ = out.flush();
                continue;
            }
        };
        let _ = writeln!(out, "{}", json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        let _ = out.flush();
    }
}

fn infer(v: &str) -> Value {
    let t = v.trim();
    if t.starts_with('[') || t.starts_with('{') {
        if let Ok(j) = serde_json::from_str(t) {
            return j;
        }
    }
    match t {
        "true" => return json!(true),
        "false" => return json!(false),
        "null" => return Value::Null,
        _ => {}
    }
    if let Ok(n) = t.parse::<i64>() {
        return json!(n);
    }
    if let Ok(n) = t.parse::<f64>() {
        if n.is_finite() {
            return json!(n);
        }
    }
    for sep in ['x', '×', ','] {
        if t.contains(sep) {
            let parts: Vec<_> = t.split(sep).map(|p| p.trim().parse::<f64>()).collect();
            if parts.len() > 1 && parts.iter().all(|p| p.is_ok()) {
                return json!(parts.into_iter().map(|p| p.unwrap()).collect::<Vec<_>>());
            }
        }
    }
    json!(v)
}

fn print_layout(l: &Value) {
    let ok = l["ok"].as_bool().unwrap_or(false);
    println!(
        "  layout {}: {} · {} trays, {} layer(s), {} mm under the lid, bed {}",
        l["strategy"].as_str().unwrap_or("?"),
        if ok { "fits" } else { "DOESN'T FIT" },
        l["trays"].as_array().map_or(0, |a| a.len()),
        l["layers"],
        l["lid_clearance_mm"],
        if l["bed_fit"].as_bool().unwrap_or(false) { "ok" } else { "TOO BIG" }
    );
    for t in l["trays"].as_array().into_iter().flatten() {
        let s: Vec<String> = t["size"].as_array().into_iter().flatten().map(|v| format!("{:.1}", v.as_f64().unwrap_or(0.0))).collect();
        let p: Vec<&str> = t["pockets"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();
        println!("    {:<14} L{} {:>22} mm  {}", t["id"].as_str().unwrap_or(""), t["layer"], s.join(" × "), p.join(", "));
    }
    for (k, label) in [("problems", "problem"), ("warnings", "warning")] {
        for r in l[k].as_array().into_iter().flatten() {
            println!("  {label}: [{}] {}", r["code"].as_str().unwrap_or(""), r["message"].as_str().unwrap_or(""));
        }
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut body = Map::new();
    let mut raw = false;
    let mut cmd = None;
    let mut i = 0;
    while i < args.len() {
        let a = std::mem::take(&mut args[i]);
        match a.as_str() {
            "-h" | "--help" | "help" => {
                print!("{HELP}");
                return;
            }
            "--json" => raw = true,
            "-g" | "--game" => {
                i += 1;
                body.insert("game".into(), json!(args.get(i).cloned().unwrap_or_default()));
            }
            _ if cmd.is_none() && !a.contains('=') => cmd = Some(a.replace('-', "_")),
            _ => {
                if let Some((k, v)) = a.split_once(":=") {
                    match serde_json::from_str(v) {
                        Ok(j) => body.insert(k.into(), j),
                        Err(e) => {
                            eprintln!("bid: {k}:= is not JSON: {e}");
                            std::process::exit(2);
                        }
                    };
                } else if let Some((k, v)) = a.split_once('=') {
                    body.insert(k.into(), infer(v));
                } else {
                    eprintln!("bid: expected key=value, got '{a}'");
                    std::process::exit(2);
                }
            }
        }
        i += 1;
    }
    let Some(cmd) = cmd else {
        print!("{HELP}");
        std::process::exit(2);
    };
    // add_photo file=<path>: send the image itself.
    if let Some(path) = body.remove("file").and_then(|v| v.as_str().map(String::from)) {
        use base64::Engine;
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            eprintln!("bid: can't read {path}: {e}");
            std::process::exit(2);
        });
        body.insert("data".into(), json!(base64::engine::general_purpose::STANDARD.encode(bytes)));
    }
    let url = server_url();
    if cmd == "mcp" {
        mcp(&url);
        return;
    }
    body.insert("cmd".into(), json!(cmd));
    let v = post(&url, Value::Object(body)).unwrap_or_else(|e| {
        eprintln!("bid: {e}");
        std::process::exit(3);
    });
    if raw {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else if v["ok"].as_bool() == Some(true) {
        println!("ok: {}", v["message"].as_str().unwrap_or(""));
        let d = &v["data"];
        if d["layout"].is_object() {
            print_layout(&d["layout"]);
        } else if let Some(ls) = d["layouts"].as_array() {
            for l in ls {
                print_layout(l);
            }
        } else if cmd == "get_state" {
            println!("  (use --json for the full state)");
        } else if !d.as_object().is_some_and(|o| o.is_empty()) {
            println!("{}", serde_json::to_string_pretty(d).unwrap());
        }
    } else {
        let r = &v["reason"];
        eprintln!("refused [{}]: {}", r["code"].as_str().unwrap_or("?"), r["message"].as_str().unwrap_or(""));
    }
    if v["ok"].as_bool() != Some(true) {
        std::process::exit(1);
    }
}
