const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
let S = null;
let editing = false;
let connSince = 0;

// ---------------------------------------------------------------- tabs
document.querySelectorAll("nav button[data-tab]").forEach((b) => {
  b.onclick = () => {
    document.querySelectorAll("nav button[data-tab]").forEach((x) => x.classList.toggle("active", x === b));
    document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("active", t.id === "tab-" + b.dataset.tab));
    document.querySelector(".scroll").scrollTop = 0;
    if (b.dataset.tab === "layout" && S) renderLayout(S);   // the canvas can only be measured once visible
  };
});

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

// ---------------------------------------------------------------- render
function render(s) {
  S = s;
  const peer = s.peers.find((p) => p.id === s.mode_peer);
  const connected = s.peers.filter((p) => p.connected);
  const st = $("statusPill");
  st.className = "pill";
  if (s.offline) st.textContent = "Disconnected";
  else if (s.paused) st.textContent = "Paused · connected";
  else if (s.mode === "forwarding") { st.classList.add("fwd"); st.textContent = "Controlling " + (peer?.name ?? "another computer"); }
  else if (s.mode === "controlled") { st.classList.add("ctl"); st.textContent = "Controlled by " + (peer?.name ?? "another computer"); }
  else st.textContent = connected.length ? `Connected · ${connected.length}` : "Searching…";
  $("reclaimBtn").classList.toggle("hidden", s.mode === "local");
  $("permBanner").classList.toggle("hidden", !(s.perms.required && !(s.perms.accessibility && s.perms.input_monitoring)));
  if (IS_ANDROID) {
    $("permText").innerHTML = "<b>One step left.</b> Turn on <b>Screen Hopper</b> under Accessibility settings (Downloaded apps) so your computers can move the pointer, tap, scroll and type on this phone. If Android says the setting is restricted, open the app’s info page, tap the ⋮ menu and choose <b>Allow restricted settings</b>.";
    $("permBtn").textContent = "Open Accessibility settings";
    $("permAcc").classList.add("hidden"); $("permInp").classList.add("hidden");
  }

  // right-hand status panel
  const pw = $("power");
  const otherPaused = connected.filter((p) => p.paused);
  pw.className = "power " + (s.offline || s.paused ? "paused" : connected.length ? "on" : "wait");
  $("heroTitle").textContent = s.offline ? "Disconnected" : s.paused ? "Paused" : connected.length ? "Connected and ready" : "Searching…";
  $("heroSub").textContent = s.offline ? "Not linked to any computer."
    : s.paused ? "Cursor stays on this computer. Clipboard and files still work."
    : connected.length ? "Hop the cursor to another screen." : "Looking for computers with your pairing key.";
  $("offlineBtn").textContent = s.offline ? "Connect" : "Disconnect";
  $("offlineBtn").classList.toggle("reconnect", s.offline);
  $("pauseBtn").textContent = s.paused ? "Resume" : "Pause";
  $("pauseBtn").classList.toggle("resume", s.paused);
  $("pauseBtn").disabled = s.offline;
  $("pauseBtn").style.opacity = s.offline ? ".45" : "1";
  // connection timer
  if (connected.length && !s.offline) { if (!connSince) connSince = Date.now(); } else connSince = 0;
  $("ipShown").textContent = s.local_ip || "-";
  // target card: the computer being controlled, else the first connected one
  const tp = peer || connected[0];
  $("tName").textContent = tp ? tp.name : "No computer yet";
  $("tSub").textContent = tp ? (tp.paused ? "Paused" : tp.connected ? (s.mode === "forwarding" && peer ? "You are controlling it" : "Connected") : "Offline") + " · " + tp.os : "Install Screen Hopper on another computer";
  $("tFlag").textContent = !tp ? "💻" : tp.os === "Windows" ? "🪟" : tp.os === "Android" ? "🤖" : tp.os === "Linux" ? "🐧" : "🍎";
  $("statMode").textContent = s.mode === "forwarding" ? "Controlling " + (peer?.name ?? "") : s.mode === "controlled" ? "Controlled by " + (peer?.name ?? "") : "This computer";
  $("statClip").textContent = s.settings.clipboard_sync ? "Synced" : "Off";
  $("ver").textContent = "v" + s.version;
  if (IS_ANDROID) {
    $("kbdState").innerHTML = s.kbd === "active" ? "<b class='ok'>Screen Hopper Keyboard is selected.</b> Typing from your computer reaches any app."
      : s.kbd === "enabled" ? "<b>Turned on, but not selected.</b> Tap “Choose keyboard” while a text box is open."
      : "<b>Not turned on yet.</b> Without it, typing only works in simple native text boxes, not in web pages.";
  }
  $("keyStat").textContent = `Keys sent ${s.keys_sent} · received ${s.keys_injected}`;
  $("code").textContent = s.settings.pairing_code;
  $("ipHelp").textContent = s.local_ip ? `This computer’s address: ${s.local_ip} (port ${s.settings.port})` : "";

  // peers
  const list = $("peerList");
  if (!s.peers.length) {
    list.innerHTML = `<div class="empty">No other computers yet. Install Screen Hopper on your other computer and enter the same pairing key.</div>`;
  } else {
    const q = $("peerSearch").value.trim().toLowerCase();
    const shown = s.peers.filter((p) => !q || p.name.toLowerCase().includes(q) || (p.os || "").toLowerCase().includes(q));
    if (!shown.length) list.innerHTML = `<div class="empty">No computer matches “${esc(q)}”.</div>`;
    else list.innerHTML = shown.map((p) => {
      const cls = p.connected ? "on" : p.error ? "bad" : p.discovered ? "found" : "";
      const ver = p.connected && p.version ? ` · v${esc(p.version)}${p.version !== s.version ? ' <span class="error">(this computer is v' + esc(s.version) + ')</span>' : ""}` : "";
      const sub = p.connected ? `Connected${p.paused ? " · <b>paused</b>" : ""} · ${esc(p.os)}${ver}`
        : p.error ? `<span class="error">${esc(p.error)}</span>`
        : p.discovered ? `Found on network · ${esc(p.os)} · connecting…` : "Offline";
      const icon = p.os === "macOS" ? "" : p.os === "Windows" ? "🪟" : p.os === "Android" ? "🤖" : p.os === "Linux" ? "🐧" : "💻";
      return `<div class="peer"><span class="flag">${icon || "🍎"}</span><div class="grow"><div class="name">${esc(p.name)}</div><div class="sub">${sub}</div></div>
        <span class="dot ${cls}"></span>
        ${p.connected ? `<button class="ghost" data-act="disc" data-id="${p.id}">Disconnect</button>` : ""}
        ${!p.connected && p.discovered && !s.offline ? `<button class="ghost" data-act="conn" data-id="${p.id}">Connect</button>` : ""}
        ${!p.connected ? `<button class="danger" data-act="forget" data-id="${p.id}">Delete</button>` : ""}</div>`;
    }).join("");
  }
  $("clearOffline").classList.toggle("hidden", !s.peers.some((p) => !p.connected));
  $("log").innerHTML = s.logs.map(esc).join("<br>") || "Nothing yet.";

  renderLayout(s);

  // files tab
  const sel = $("filePeer");
  const cur = sel.value;
  sel.innerHTML = `<option value="">All connected computers</option>` + connected.map((p) => `<option value="${p.id}">${esc(p.name)}</option>`).join("");
  sel.value = connected.some((p) => p.id === cur) ? cur : "";
  $("recvDir").textContent = s.settings.receive_dir;

  if (!editing) fillSettings(s.settings);
}

// ---- layout canvas -------------------------------------------------------
let drag = null;          // { id, startX, startY, ox, oy }
let dragPos = {};         // live positions while dragging
const peerPos = (p, s) => dragPos[p.id] ?? (p.placed ? { x: p.x, y: p.y } : { x: unionW(s.my_mons), y: 0 });
const unionW = (rs) => Math.max(0, ...rs.map((r) => r.x + r.w));

function layoutRects(s) {
  const out = [];
  s.my_mons.forEach((r, i) => out.push({ ...r, me: true, label: s.my_mons.length > 1 ? `This computer · ${i + 1}` : "This computer" }));
  s.peers.filter((p) => p.mons && p.mons.length).forEach((p) => {
    const pos = peerPos(p, s);
    p.mons.forEach((r, i) => out.push({ x: r.x + pos.x, y: r.y + pos.y, w: r.w, h: r.h, id: p.id, on: p.connected, label: p.mons.length > 1 ? `${p.name} · ${i + 1}` : p.name }));
  });
  return out;
}

function renderLayout(s) {
  if (drag) return;
  const stage = $("stage");
  const rects = layoutRects(s);
  const W = stage.clientWidth || 640, H = stage.clientHeight || 360;
  const minX = Math.min(...rects.map((r) => r.x)), maxX = Math.max(...rects.map((r) => r.x + r.w));
  const minY = Math.min(...rects.map((r) => r.y)), maxY = Math.max(...rects.map((r) => r.y + r.h));
  const padX = (maxX - minX) * 0.35, padY = (maxY - minY) * 0.35;
  const scale = Math.min(W / (maxX - minX + 2 * padX), H / (maxY - minY + 2 * padY));
  const offX = (W - (maxX - minX) * scale) / 2 - minX * scale;
  const offY = (H - (maxY - minY) * scale) / 2 - minY * scale;
  stage._t = { scale, offX, offY };
  const hasPeer = s.peers.some((p) => p.mons && p.mons.length);
  stage.innerHTML = rects.map((r) => `<div class="mon ${r.me ? "me" : "peer"} ${r.on ? "on" : ""}" ${r.id ? `data-id="${r.id}"` : ""}
      style="left:${r.x * scale + offX}px;top:${r.y * scale + offY}px;width:${r.w * scale}px;height:${r.h * scale}px">
      <span>${esc(r.label)}</span><small>${Math.round(r.w)}×${Math.round(r.h)}</small></div>`).join("") +
    (hasPeer ? "" : `<div class="stage-empty">Connect another computer to arrange it here.</div>`);
}

function snapPos(id, x, y, s, scale) {
  const peer = s.peers.find((p) => p.id === id);
  // Snap against this computer's screens AND every other computer's screens.
  const others = [...s.my_mons];
  s.peers.filter((p) => p.id !== id && p.mons && p.mons.length).forEach((p) => {
    const pos = peerPos(p, s);
    p.mons.forEach((r) => others.push({ x: r.x + pos.x, y: r.y + pos.y, w: r.w, h: r.h }));
  });
  const thr = 18 / scale;
  let bestX = null, bestY = null;
  for (const a of others) for (const b of peer.mons) {
    const bx = b.x + x, by = b.y + y;
    for (const d of [a.x + a.w - bx, a.x - (bx + b.w), a.x - bx, a.x + a.w - (bx + b.w)])
      if (Math.abs(d) < thr && (bestX === null || Math.abs(d) < Math.abs(bestX))) bestX = d;
    for (const d of [a.y + a.h - by, a.y - (by + b.h), a.y - by, a.y + a.h - (by + b.h)])
      if (Math.abs(d) < thr && (bestY === null || Math.abs(d) < Math.abs(bestY))) bestY = d;
  }
  return { x: x + (bestX ?? 0), y: y + (bestY ?? 0) };
}

$("stage").addEventListener("pointerdown", (e) => {
  const el = e.target.closest(".mon.peer");
  if (!el || !S) return;
  const p = S.peers.find((q) => q.id === el.dataset.id);
  const pos = peerPos(p, S);
  drag = { id: p.id, sx: e.clientX, sy: e.clientY, ox: pos.x, oy: pos.y };
  $("stage").setPointerCapture(e.pointerId);
  e.preventDefault();
});
$("stage").addEventListener("pointermove", (e) => {
  if (!drag) return;
  const { scale } = $("stage")._t;
  const raw = { x: drag.ox + (e.clientX - drag.sx) / scale, y: drag.oy + (e.clientY - drag.sy) / scale };
  const pos = snapPos(drag.id, raw.x, raw.y, S, scale);
  dragPos[drag.id] = pos;
  const { offX, offY } = $("stage")._t;
  const p = S.peers.find((q) => q.id === drag.id);
  const els = [...$("stage").querySelectorAll(`.mon.peer[data-id="${drag.id}"]`)];
  p.mons.forEach((r, i) => {
    if (!els[i]) return;
    els[i].style.left = (r.x + pos.x) * scale + offX + "px";
    els[i].style.top = (r.y + pos.y) * scale + offY + "px";
  });
});
const endDrag = async () => {
  if (!drag) return;
  const id = drag.id, pos = dragPos[id];
  drag = null;
  if (pos) { const p = S.peers.find((q) => q.id === id); p.x = pos.x; p.y = pos.y; p.placed = true; await invoke("set_layout", { id, x: pos.x, y: pos.y }); }
  delete dragPos[id];
  renderLayout(S);
};
$("stage").addEventListener("pointerup", endDrag);
$("stage").addEventListener("pointercancel", endDrag);

$("offlineBtn").onclick = () => invoke("set_offline", { offline: !S.offline }).then(() => invoke("get_state").then(render));
const togglePause = () => invoke("set_paused", { paused: !S.paused }).then(() => invoke("get_state").then(render));
$("power").onclick = togglePause;
$("pauseBtn").onclick = togglePause;
$("quitBtn").onclick = () => invoke("quit_app");
let pinned = false;
$("pinBtn").onclick = () => { pinned = !pinned; $("pinBtn").classList.toggle("on", pinned); invoke("set_pinned", { on: pinned }); };
// Timer
setInterval(() => {
  if (!connSince) { $("connTime").textContent = "--:--:--"; return; }
  const t = Math.floor((Date.now() - connSince) / 1000);
  const p = (n) => String(n).padStart(2, "0");
  $("connTime").textContent = `${p(Math.floor(t / 3600))}:${p(Math.floor(t / 60) % 60)}:${p(t % 60)}`;
}, 1000);

// ---------------------------------------------------------------- pause shortcut recorder
// KeyboardEvent.code -> USB HID usage (the code the app works in on both Mac and Windows)
const CODE2HID = {
  Enter: 0x28, Escape: 0x29, Backspace: 0x2a, Tab: 0x2b, Space: 0x2c, Minus: 0x2d, Equal: 0x2e,
  BracketLeft: 0x2f, BracketRight: 0x30, Backslash: 0x31, Semicolon: 0x33, Quote: 0x34, Backquote: 0x35,
  Comma: 0x36, Period: 0x37, Slash: 0x38, Insert: 0x49, Home: 0x4a, PageUp: 0x4b, Delete: 0x4c, End: 0x4d,
  PageDown: 0x4e, ArrowRight: 0x4f, ArrowLeft: 0x50, ArrowDown: 0x51, ArrowUp: 0x52,
  ControlLeft: 0xe0, ShiftLeft: 0xe1, AltLeft: 0xe2, MetaLeft: 0xe3, ControlRight: 0xe4, ShiftRight: 0xe5, AltRight: 0xe6, MetaRight: 0xe7,
};
for (let i = 0; i < 26; i++) CODE2HID["Key" + String.fromCharCode(65 + i)] = 0x04 + i;
for (let i = 1; i <= 9; i++) CODE2HID["Digit" + i] = 0x1d + i;
CODE2HID.Digit0 = 0x27;
for (let i = 1; i <= 12; i++) CODE2HID["F" + i] = 0x39 + i;
const HID2CODE = Object.fromEntries(Object.entries(CODE2HID).map(([k, v]) => [v, k]));
const MOD_OF = { ControlLeft: "ctrl", ControlRight: "ctrl", ShiftLeft: "shift", ShiftRight: "shift", AltLeft: "alt", AltRight: "alt", MetaLeft: "meta", MetaRight: "meta" };
const MOD_BY_KEY = { Control: "ctrl", Shift: "shift", Alt: "alt", Meta: "meta" };
const MOD_LEFT = { ctrl: "ControlLeft", shift: "ShiftLeft", alt: "AltLeft", meta: "MetaLeft" };
// Some keyboards/browsers leave e.code empty for modifiers, so fall back to e.key.
const modGroupOf = (e) => MOD_OF[e.code] || MOD_BY_KEY[e.key] || null;
const IS_MAC = document.documentElement.classList.contains("mac");
const IS_ANDROID = document.documentElement.classList.contains("android");
const MOD_NAME = { ctrl: "Ctrl", alt: IS_MAC ? "Option" : "Alt", shift: "Shift", meta: IS_MAC ? "Cmd" : "Win" };
function keyName(hid) {
  const code = HID2CODE[hid] || "?";
  if (MOD_OF[code]) return MOD_NAME[MOD_OF[code]];
  return code.replace(/^Key/, "").replace(/^Digit/, "").replace(/^Arrow/, "") ;
}
function hkLabel(hk) {
  if (!hk || !hk.key) return "Not set";
  if (hk.double) return "Double-tap " + keyName(hk.key);
  return [...["ctrl", "alt", "shift", "meta"].filter((m) => hk.mods.includes(m)).map((m) => MOD_NAME[m]), keyName(hk.key)].join(" + ");
}
let curHk = { enabled: true, double: false, mods: ["ctrl", "alt"], key: 0x13 };
let recording = null;
function stopRecording(saved) {
  recording = null;
  $("hkBox").classList.remove("rec");
  $("hkBox").textContent = hkLabel(curHk);
  $("hkHint").textContent = saved ? "Saved. Press it anywhere to pause or resume this computer." : "Click to change. Press a shortcut, or tap one key twice.";
  invoke("set_hotkey_recording", { on: false });
}
function finishRecording(hk) {
  curHk = { enabled: true, ...hk };
  $("sHkOn").checked = true;
  stopRecording(true);
  saveSettings(true);
}
$("hkBox").onclick = () => {
  if (recording) return stopRecording(false);
  recording = { lastKey: null, modDown: null, lastMod: null };
  invoke("set_hotkey_recording", { on: true });
  $("hkBox").classList.add("rec");
  $("hkBox").textContent = "Press a shortcut…";
  $("hkHint").textContent = "e.g. Ctrl + Alt + P, or tap Ctrl twice. Esc cancels.";
};
window.addEventListener("keydown", (e) => {
  if (!recording) return;
  e.preventDefault(); e.stopPropagation();
  if (e.repeat) return;
  const now = Date.now();
  const hid = CODE2HID[e.code];
  const grp = modGroupOf(e);
  if (grp) { recording.modDown = { group: grp, t: now, other: false }; return; }
  if (recording.modDown) recording.modDown.other = true;
  const mods = [e.ctrlKey && "ctrl", e.altKey && "alt", e.shiftKey && "shift", e.metaKey && "meta"].filter(Boolean);
  if (e.code === "Escape" && !mods.length) return stopRecording(false);
  if (!hid) { $("hkHint").textContent = "That key can't be used. Try another."; return; }
  if (mods.length) return finishRecording({ double: false, mods, key: hid });
  // A plain key needs a second quick tap.
  if (recording.lastKey && recording.lastKey.hid === hid && now - recording.lastKey.t < 450) return finishRecording({ double: true, mods: [], key: hid });
  recording.lastKey = { hid, t: now };
  $("hkHint").textContent = "Tap it again quickly for a double-tap, or hold Ctrl / Alt / Shift with it.";
}, true);
window.addEventListener("keyup", (e) => {
  if (!recording) return;
  e.preventDefault(); e.stopPropagation();
  const g = modGroupOf(e);
  const md = recording.modDown;
  if (!g || !md || md.group !== g) return;
  recording.modDown = null;
  if (md.other || Date.now() - md.t > 350) return;             // was used as a modifier, not tapped
  const now = Date.now();
  if (recording.lastMod && recording.lastMod.group === g && now - recording.lastMod.t < 450) {
    return finishRecording({ double: true, mods: [], key: CODE2HID[MOD_LEFT[g]] });
  }
  recording.lastMod = { group: g, t: now };
  $("hkHint").textContent = "Tap " + MOD_NAME[g] + " once more to use a double-tap.";
}, true);

function fillSettings(c) {
  $("sName").value = c.device_name;
  $("sCode").value = c.pairing_code;
  $("sLogin").checked = c.launch_at_login;
  curHk = { ...c.pause_hotkey };
  $("sHkOn").checked = curHk.enabled;
  if (!recording) $("hkBox").textContent = hkLabel(curHk);
  $("sSwap").checked = c.swap_cmd_ctrl;
  $("sRevHere").checked = c.reverse_scroll_here;
  $("sClip").checked = c.clipboard_sync;
  $("sClipFiles").checked = c.clipboard_files;
  $("sFiles").checked = c.file_transfer;
  $("sDim").checked = c.dim_enabled;
  $("sDimLevel").value = Math.round(c.dim_level * 100);
  $("sDir").value = c.receive_dir;
}

// ---------------------------------------------------------------- actions
$("peerList").addEventListener("click", (e) => {
  const b = e.target.closest("button[data-act]");
  if (!b) return;
  if (b.dataset.act === "disc") invoke("disconnect", { id: b.dataset.id });
  if (b.dataset.act === "conn") invoke("connect_peer", { id: b.dataset.id });
  if (b.dataset.act === "forget") invoke("forget_peer", { id: b.dataset.id }).catch((e) => alert(e));
});
$("clearOffline").onclick = () => { if (confirm("Delete all computers that are not connected right now?")) invoke("clear_offline"); };
$("kbdEnable").onclick = () => invoke("keyboard_action", { action: "enable" });
$("kbdPick").onclick = () => invoke("keyboard_action", { action: "pick" });
$("peerSearch").addEventListener("input", () => S && render(S));
$("copyCode").onclick = async () => { try { await navigator.clipboard.writeText(S.settings.pairing_code); $("copyCode").textContent = "Copied"; setTimeout(() => ($("copyCode").textContent = "Copy"), 1200); } catch {} };
$("newCode").onclick = async () => { if (confirm("Make a new pairing code? Connected computers will disconnect until you enter the new code on them.")) await invoke("regenerate_code"); };
$("ipBtn").onclick = async () => {
  $("ipErr").textContent = "";
  try { await invoke("connect_addr", { addr: $("ipInput").value }); $("ipInput").value = ""; }
  catch (e) { $("ipErr").textContent = String(e); }
};
$("permBtn").onclick = () => invoke("request_permissions");
$("permAcc").onclick = () => invoke("open_privacy_settings", { pane: "accessibility" });
$("permInp").onclick = () => invoke("open_privacy_settings", { pane: "input" });
$("reclaimBtn").onclick = () => invoke("reclaim");
$("openRecv").onclick = () => invoke("open_receive_dir");
$("logBtn").onclick = () => invoke("open_log_dir");

["sName","sCode","sLogin","sHkOn","sSwap","sRevHere","sClip","sClipFiles","sFiles","sDim","sDimLevel","sDir"].forEach((id) => {
  $(id).addEventListener("input", () => { editing = true; $("saved").textContent = ""; });
});
async function saveSettings(quiet) {
  await invoke("save_settings", { patch: {
    device_name: $("sName").value.trim() || S.settings.device_name,
    pairing_code: $("sCode").value.trim().toUpperCase() || S.settings.pairing_code,
    launch_at_login: $("sLogin").checked,
    pause_hotkey: { ...curHk, enabled: $("sHkOn").checked },
    swap_cmd_ctrl: $("sSwap").checked,
    reverse_scroll_here: $("sRevHere").checked,
    clipboard_sync: $("sClip").checked,
    clipboard_files: $("sClipFiles").checked,
    file_transfer: $("sFiles").checked,
    dim_enabled: $("sDim").checked,
    dim_level: $("sDimLevel").value / 100,
    receive_dir: $("sDir").value.trim() || S.settings.receive_dir,
  }});
  editing = false;
  $("saved").textContent = quiet ? "Applied." : "Saved. A new computer name shows to others after restart.";
}
$("saveBtn").onclick = () => saveSettings(false);
// Switches and sliders apply the moment you change them.
["sLogin","sHkOn","sSwap","sRevHere","sClip","sClipFiles","sFiles","sDim","sDimLevel"].forEach((id) => $(id).addEventListener("change", () => saveSettings(true)));

// ---------------------------------------------------------------- files
async function sendPaths(paths) {
  if (!paths || !paths.length) return;
  const peer = $("filePeer").value || null;
  try { await invoke("send_files", { peer, paths }); } catch (e) { alert(e); }
}
async function pick(opts) {
  await invoke("set_keep_open", { on: true });          // the popover would hide when the dialog takes focus
  try { return await window.__TAURI__.dialog.open(opts); } finally { await invoke("set_keep_open", { on: false }); }
}
$("pickFiles").onclick = async () => {
  if (IS_ANDROID) { const r = await invoke("pick_files"); if (r && r.length) sendPaths(r); return; }
  const r = await pick({ multiple: true, directory: false }); if (r) sendPaths(Array.isArray(r) ? r : [r]);
};
$("sendClip").onclick = async () => { const ok = await invoke("send_clipboard_now"); $("sendClip").textContent = ok ? "Sent" : "Nothing to send: copy some text first"; setTimeout(() => ($("sendClip").textContent = "Send this phone’s clipboard to the other computers"), 2000); };
$("pickFolder").onclick = async () => { const r = await pick({ multiple: false, directory: true }); if (r) sendPaths([r]); };
try {
  window.__TAURI__.webview.getCurrentWebview().onDragDropEvent((ev) => {
    const t = ev.payload.type;
    if (t === "over" || t === "enter") $("drop").classList.add("over");
    else if (t === "leave") $("drop").classList.remove("over");
    else if (t === "drop") { $("drop").classList.remove("over"); sendPaths(ev.payload.paths); }
  });
} catch (e) { console.warn("drag/drop unavailable", e); }

let hideTimer;
listen("transfer", (e) => {
  const p = e.payload;
  const bar = $("xferBar");
  bar.classList.remove("hidden");
  const pct = p.total ? Math.min(100, (p.done / p.total) * 100) : 100;
  $("xferFill").style.width = pct + "%";
  $("xferText").textContent = p.finished ? (p.dir === "send" ? "Sent " : "Received ") + p.name : (p.dir === "send" ? "Sending " : "Receiving ") + p.name + " · " + Math.round(pct) + "%";
  clearTimeout(hideTimer);
  if (p.finished) hideTimer = setTimeout(() => bar.classList.add("hidden"), 4000);
});

// ---------------------------------------------------------------- boot
listen("state", (e) => render(e.payload));
invoke("get_state").then(render);
setInterval(() => invoke("get_state").then(render), 1000);
