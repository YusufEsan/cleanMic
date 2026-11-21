"use strict";

// ------------------------------------------------------------------ Tauri köprüsü
// Tauri dışında (tarayıcıda) açılırsa basit bir sahte backend ile çalışır, tasarım önizlemesi için.
const T = window.__TAURI__;
const invoke = T ? T.core.invoke : mockInvoke;
const listen = T ? T.event.listen : mockListen;

const $ = (s) => document.querySelector(s);
const el = (tag, cls, html) => { const e = document.createElement(tag); if (cls) e.className = cls; if (html != null) e.innerHTML = html; return e; };

let dsp = {};
let presetNames = [];
let pads = [];
let status = {};
let underBase = 0;
let monUnderBase = 0;

// ------------------------------------------------------------------ pedal tanımları
const fmt = {
  db: (v) => `${v > 0 ? "+" : ""}${v.toFixed(v % 1 ? 1 : 0)} dB`,
  ms: (v) => `${Math.round(v)} ms`,
  pct: (v) => `${Math.round(v * 100)}%`,
  ratio: (v) => `${v.toFixed(1)}:1`,
  st: (v) => `${v > 0 ? "+" : ""}${Math.round(v)} st`,
  tone: (v) => (Math.abs(v) < 0.05 ? "düz" : `${v > 0 ? "+" : ""}${Math.round(v * 100)}`),
};

const PEDALS = [
  { name: "Gate", sub: "NOISE GATE", color: "#4A453E", on: "gate_on",
    knobs: [{ p: "gate_thresh_db", l: "EŞİK", min: -80, max: -20, step: 1, f: fmt.db, def: -50 },
            { p: "gate_release_ms", l: "BIRAK", min: 20, max: 500, step: 5, f: fmt.ms, def: 150 }] },
  { name: "Temiz AI", sub: "RNNOISE", color: "#1F5F5B", on: "ai_on", vad: true,
    knobs: [{ p: "ai_strength", l: "GÜÇ", min: 0, max: 1, step: 0.01, f: fmt.pct, def: 0.9 }] },
  { name: "Kompresör", sub: "LEVELER", color: "#3E4A63", on: "comp_on", gr: true,
    knobs: [{ p: "comp_ratio", l: "ORAN", min: 1, max: 10, step: 0.1, f: fmt.ratio, def: 3 },
            { p: "comp_makeup_db", l: "KAZANÇ", min: 0, max: 18, step: 0.5, f: fmt.db, def: 6 }] },
  { name: "EQ", sub: "3 BANT", color: "#3D5A3A", on: "eq_on",
    knobs: [{ p: "eq_low_db", l: "BAS", min: -12, max: 12, step: 0.5, f: fmt.db, def: 0 },
            { p: "eq_mid_db", l: "ORTA", min: -12, max: 12, step: 0.5, f: fmt.db, def: 0 },
            { p: "eq_high_db", l: "TİZ", min: -12, max: 12, step: 0.5, f: fmt.db, def: 0 }] },
  { name: "De-esser", sub: "S YUMUŞATICI", color: "#4F3F5E", on: "deess_on",
    knobs: [{ p: "deess_thresh_db", l: "EŞİK", min: -50, max: -10, step: 1, f: fmt.db, def: -30 },
            { p: "deess_amount", l: "MİKTAR", min: 0, max: 1, step: 0.01, f: fmt.pct, def: 0.6 }] },
  { name: "Ses FX", sub: "PITCH · TON", color: "#6B3F6E", on: "fx_on",
    knobs: [{ p: "fx_pitch", l: "PITCH", min: -12, max: 12, step: 1, f: fmt.st, def: 0 },
            { p: "fx_tone", l: "TON", min: -1, max: 1, step: 0.05, f: fmt.tone, def: 0 }] },
  { name: "Reverb", sub: "ODA · SALON", color: "#7A4A1E", on: "rev_on",
    knobs: [{ p: "rev_mix", l: "MİKTAR", min: 0, max: 1, step: 0.01, f: fmt.pct, def: 0.2 },
            { p: "rev_size", l: "BOYUT", min: 0, max: 1, step: 0.01, f: fmt.pct, def: 0.55 }] },
  { name: "Delay", sub: "EKO", color: "#5E5220", on: "dly_on",
    knobs: [{ p: "dly_ms", l: "SÜRE", min: 20, max: 800, step: 10, f: fmt.ms, def: 280 },
            { p: "dly_fb", l: "TEKRAR", min: 0, max: 0.9, step: 0.01, f: fmt.pct, def: 0.3 }] },
  { name: "Limiter", sub: "ANTİ-PATLAMA", color: "#7A2A1E", on: "lim_on", gr: true,
    knobs: [{ p: "lim_ceiling_db", l: "TAVAN", min: -12, max: 0, step: 0.5, f: fmt.db, def: -1 },
            { p: "lim_release_ms", l: "HIZ", min: 20, max: 300, step: 5, f: fmt.ms, def: 80 }] },
];

const knobEls = {}; // param -> {svgLine, valEl, def}
const pedalEls = []; // {root, led, gr, def}

// ------------------------------------------------------------------ dsp gönderimi (karede bir kez)
let pending = {};
let flushQueued = false;
function setDsp(patch) {
  Object.assign(dsp, patch);
  Object.assign(pending, patch);
  if (!flushQueued) {
    flushQueued = true;
    requestAnimationFrame(async () => {
      const p = pending; pending = {}; flushQueued = false;
      try { dsp = await invoke("set_dsp", { patch: p }); } catch (e) { console.error(e); }
    });
  }
}

// ------------------------------------------------------------------ knob
function knobAngle(k, v) { return -135 + 270 * (v - k.min) / (k.max - k.min); }

function buildKnob(k) {
  const w = el("div", "knob");
  w.tabIndex = 0;
  w.setAttribute("role", "slider");
  w.setAttribute("aria-label", k.l);
  w.setAttribute("aria-valuemin", k.min);
  w.setAttribute("aria-valuemax", k.max);
  w.innerHTML = `
    <svg width="44" height="44" viewBox="0 0 46 46" aria-hidden="true">
      <circle cx="23" cy="23" r="20" fill="#1C1915" stroke="#000"/>
      <circle cx="23" cy="23" r="15" fill="#2E2924"/>
      <line x1="23" y1="23" x2="23" y2="8" stroke="#F2E8D5" stroke-width="3" stroke-linecap="round"/>
    </svg>
    <span class="k-label">${k.l}</span><span class="k-val"></span>`;
  const line = w.querySelector("line");
  const val = w.querySelector(".k-val");
  knobEls[k.p] = { k, w, line, val };

  const apply = (v) => {
    v = Math.min(k.max, Math.max(k.min, Math.round(v / k.step) * k.step));
    if (v === dsp[k.p]) return;
    setDsp({ [k.p]: v });
    paintKnob(k.p);
  };
  let startY = 0, startV = 0;
  w.addEventListener("pointerdown", (e) => {
    w.setPointerCapture(e.pointerId);
    startY = e.clientY; startV = dsp[k.p]; w.classList.add("drag");
  });
  w.addEventListener("pointermove", (e) => {
    if (!w.hasPointerCapture(e.pointerId)) return;
    const range = k.max - k.min;
    const sens = e.shiftKey ? 600 : 160; // Shift ile ince ayar
    apply(startV + (startY - e.clientY) / sens * range);
  });
  const end = () => w.classList.remove("drag");
  w.addEventListener("pointerup", end);
  w.addEventListener("pointercancel", end);
  w.addEventListener("dblclick", () => apply(k.def));
  w.addEventListener("wheel", (e) => { e.preventDefault(); apply(dsp[k.p] + (e.deltaY < 0 ? 1 : -1) * k.step * (e.shiftKey ? 1 : 3)); }, { passive: false });
  w.addEventListener("keydown", (e) => {
    const big = (k.max - k.min) / 10;
    const d = { ArrowUp: k.step, ArrowRight: k.step, ArrowDown: -k.step, ArrowLeft: -k.step, PageUp: big, PageDown: -big }[e.key];
    if (d != null) { e.preventDefault(); apply(dsp[k.p] + d); }
    if (e.key === "Home") apply(k.min);
    if (e.key === "End") apply(k.max);
  });
  return w;
}

function paintKnob(p) {
  const o = knobEls[p];
  if (!o || dsp[p] == null) return;
  o.line.setAttribute("transform", `rotate(${knobAngle(o.k, dsp[p])} 23 23)`);
  o.val.textContent = o.k.f(dsp[p]);
  o.w.setAttribute("aria-valuenow", dsp[p]);
  o.w.setAttribute("aria-valuetext", o.k.f(dsp[p]));
}

// ------------------------------------------------------------------ pedallar
function buildBoard() {
  const board = $("#board");
  board.innerHTML = "";
  for (const def of PEDALS) {
    const root = el("div", "pedal" + (def.knobs.length > 2 ? " k3" : ""));
    root.style.background = def.color;
    const knobs = el("div", "knobs");
    def.knobs.forEach((k) => knobs.appendChild(buildKnob(k)));
    root.appendChild(knobs);
    root.appendChild(el("div", null, `<div class="p-name">${def.name}</div><div class="p-sub">${def.sub}</div>`));
    const meta = el("div", "p-meta");
    const led = el("span", "led");
    meta.appendChild(led);
    let gr = null;
    if (def.gr || def.vad) { gr = el("span", "gr"); meta.appendChild(gr); }
    root.appendChild(meta);
    const foot = el("button", "foot");
    foot.setAttribute("aria-label", `${def.name} pedalını aç/kapat`);
    foot.addEventListener("click", () => { setDsp({ [def.on]: !dsp[def.on] }); paintPedals(); });
    root.appendChild(foot);
    board.appendChild(root);
    pedalEls.push({ def, root, led, gr, foot });
  }
}

function paintPedals() {
  for (const p of pedalEls) {
    const on = !!dsp[p.def.on];
    p.root.classList.toggle("off", !on);
    p.led.classList.toggle("on", on && !dsp.mute);
    p.foot.setAttribute("aria-pressed", on);
  }
  Object.keys(knobEls).forEach(paintKnob);
}

// ------------------------------------------------------------------ presetler
// ------------------------------------------------------------------ temalı onay / isim penceresi ve menü
/** Onay sorar. danger:true ise onay düğmesi kırmızı olur. Promise<boolean> döner. */
function ask({ title, text = "", ok = "Tamam", cancel = "Vazgeç", danger = false }) {
  return openAsk({ title, text, ok, cancel, danger, input: null }).then((r) => r !== null);
}
/** Metin ister (ör. yeniden adlandırma). Promise<string|null> döner. */
function askText({ title, text = "", value = "", ok = "Kaydet", placeholder = "" }) {
  return openAsk({ title, text, ok, cancel: "Vazgeç", danger: false, input: { value, placeholder } });
}
function openAsk(o) {
  const dlg = $("#dlg-ask"), inp = $("#ask-input"), okBtn = $("#ask-ok");
  $("#ask-title").textContent = o.title;
  $("#ask-text").textContent = o.text;
  $("#ask-text").hidden = !o.text;
  $("#ask-icon").textContent = o.danger ? "!" : o.input ? "✎" : "?";
  $("#ask-icon").className = "ask-icon" + (o.danger ? " danger" : "");
  okBtn.textContent = o.ok;
  okBtn.className = "btn " + (o.danger ? "btn-danger" : "btn-hot");
  $("#ask-cancel").textContent = o.cancel;
  inp.hidden = !o.input;
  dlg.querySelector(".modal-body").hidden = !o.text && !o.input;
  if (o.input) { inp.value = o.input.value; inp.placeholder = o.input.placeholder; }
  dlg.returnValue = "";
  dlg.showModal();
  if (o.input) { inp.focus(); inp.select(); } else okBtn.focus();
  return new Promise((resolve) => {
    const onClose = () => {
      dlg.removeEventListener("close", onClose);
      if (dlg.returnValue !== "ok") return resolve(null);
      resolve(o.input ? inp.value.trim() || null : "ok");
    };
    dlg.addEventListener("close", onClose);
  });
}

/** Sağ tık / ⋯ menüsü. items: [{label, fn, danger?, hint?}] ya da "-" ayırıcı */
function showMenu(x, y, items) {
  const m = $("#ctx-menu");
  m.innerHTML = "";
  for (const it of items) {
    if (it === "-") { m.appendChild(el("div", "menu-sep")); continue; }
    const b = el("button", "menu-item" + (it.danger ? " danger" : ""), `<span>${escapeHtml(it.label)}</span>${it.hint ? `<small>${escapeHtml(it.hint)}</small>` : ""}`);
    b.type = "button";
    b.setAttribute("role", "menuitem");
    b.addEventListener("click", () => { hideMenu(); it.fn(); });
    m.appendChild(b);
  }
  m.hidden = false;
  const r = m.getBoundingClientRect();
  m.style.left = `${Math.min(x, innerWidth - r.width - 8)}px`;
  m.style.top = `${Math.min(y, innerHeight - r.height - 8)}px`;
  m.querySelector("button")?.focus();
}
function hideMenu() { $("#ctx-menu").hidden = true; }
document.addEventListener("pointerdown", (e) => { if (!e.target.closest("#ctx-menu")) hideMenu(); });
document.addEventListener("keydown", (e) => { if (e.key === "Escape") hideMenu(); });
window.addEventListener("blur", hideMenu);

const CUSTOM_BASE = 100;
const PTABS = [
  { id: "clean", name: "Temiz", from: 0, to: 4, desc: "Gürültüsüz doğal ses ve stüdyo kalitesi" },
  { id: "char", name: "Karakter", from: 4, to: 12, desc: "Perde ve tını değiştiren eğlenceli sesler" },
  { id: "dev", name: "Cihaz", from: 12, to: 16, desc: "Radyo, telefon, telsiz, megafon" },
  { id: "env", name: "Ortam", from: 16, to: 20, desc: "Yankılı mekânlar ve retro" },
  { id: "mine", name: "Benim", custom: true, desc: "Kendi kaydettiğin modlar" },
];
const PRESET_DESC = [
  "Sadece temizlik, efekt yok", "Sıcak alt, parlak üst, sıkı kompresör", "Radyo DJ tokluğu, yoğun kompresyon", "Net ve doğal konuşma",
  "Metalik, mekanik", "5 ton aşağı, karanlık", "Bir oktav yukarı", "Ölçülü ince ses",
  "İnce ve parlak", "Oktav aşağı, hırıltılı", "Titreyen metalik ses", "Dalgalanan, yankılı",
  "Eski AM radyo cızırtısı", "Klasik telefon hattı", "Dar bant, sert, cızırtılı", "Korna, aşırı kırpma",
  "Geniş taş yankı", "Spiker hoparlörü + salon", "Boğuk ve dalgalı", "Retro oyun sesi",
];
let customPresets = [];
let ptab = null;

function tabOfPreset() {
  if (dsp.preset >= CUSTOM_BASE && dsp.preset < 255) return "mine";
  const t = PTABS.find((t) => !t.custom && dsp.preset >= t.from && dsp.preset < t.to);
  return t ? t.id : "clean";
}

function buildPresetTabs() {
  const bar = $("#ptabs");
  bar.innerHTML = "";
  for (const t of PTABS) {
    const b = el("button", "ptab", t.name);
    b.type = "button";
    b.setAttribute("role", "tab");
    b.dataset.tab = t.id;
    b.title = t.desc;
    b.addEventListener("click", () => { ptab = t.id; buildPresets(); });
    bar.appendChild(b);
  }
}

function hotkeyFor(i) {
  return i < 12 ? { short: `F${i + 1}`, long: `Ctrl+Alt+F${i + 1}` } : { short: `⇧F${i - 11}`, long: `Ctrl+Alt+Shift+F${i - 11}` };
}

function buildPresets() {
  if (!ptab) ptab = tabOfPreset();
  document.querySelectorAll(".ptab").forEach((b) => b.setAttribute("aria-selected", b.dataset.tab === ptab));
  const box = $("#presets");
  box.innerHTML = "";
  const tab = PTABS.find((t) => t.id === ptab);
  // 4 veya daha az kart varsa iki satırı dolduran açıklamalı büyük kartlar
  const count = tab.custom ? customPresets.length + 1 : tab.to - tab.from;
  const few = count <= 4;
  box.classList.toggle("few", few);
  if (tab.custom) {
    customPresets.forEach((c, i) => {
      const b = el("button", "preset custom", `<span class="pl"></span><span class="pt"><b>${escapeHtml(c.name)}</b><em>Kendi ayarın</em></span>`);
      b.dataset.idx = CUSTOM_BASE + i;
      b.addEventListener("click", async () => { dsp = await invoke("apply_custom", { index: i }); paintAll(); });
      const more = el("span", "del", "⋯");
      more.setAttribute("role", "button");
      more.setAttribute("aria-label", `${c.name} modu için seçenekler`);
      more.title = "Seçenekler";
      more.addEventListener("click", (e) => { e.stopPropagation(); const r = more.getBoundingClientRect(); customMenu(i, r.left, r.bottom + 4); });
      b.addEventListener("contextmenu", (e) => { e.preventDefault(); customMenu(i, e.clientX, e.clientY); });
      b.appendChild(more);
      box.appendChild(b);
    });
    const add = el("button", "preset add", `<span class="pt"><b>＋ Şu anki sesi kaydet</b><em>Pedal ayarlarını isimle sakla</em></span>`);
    add.addEventListener("click", openSaveForm);
    box.appendChild(add);
    $("#preset-hint").textContent = customPresets.length
      ? "Pedalları istediğin gibi ayarla, sonra kaydet. Silmek için × ."
      : "Pedalları ve düğmeleri istediğin gibi ayarla, sonra kaydet; tek tıkla geri çağırırsın.";
  } else {
    for (let i = tab.from; i < tab.to; i++) {
      const hk = hotkeyFor(i);
      const b = el("button", "preset", `<span class="pl"></span><span class="pt"><b>${presetNames[i]}</b><em>${PRESET_DESC[i] || ""}</em></span><small>${hk.short}</small>`);
      b.dataset.idx = i;
      b.title = hk.long;
      b.addEventListener("click", async () => { dsp = await invoke("set_preset", { index: i }); paintAll(); });
      box.appendChild(b);
    }
    $("#preset-hint").textContent = `${tab.desc} · Ctrl+Alt+F1…F12, Ctrl+Alt+Shift+F1…F8`;
  }
  paintPresets();
}

function paintPresets() {
  [...$("#presets").children].forEach((b) => {
    if (b.dataset.idx != null) b.setAttribute("aria-pressed", dsp.fx_on && dsp.preset === +b.dataset.idx);
  });
  // seçili mod başka sekmedeyse o sekmede turuncu nokta
  document.querySelectorAll(".ptab").forEach((b) => {
    const t = PTABS.find((x) => x.id === b.dataset.tab);
    const active = t.custom ? dsp.preset >= CUSTOM_BASE && dsp.preset < 255 : dsp.preset >= t.from && dsp.preset < t.to;
    b.classList.toggle("has-active", active && !!dsp.fx_on);
  });
}

function customMenu(i, x, y) {
  const c = customPresets[i];
  const done = (r) => { customPresets = r.custom; dsp = r.dsp; paintAll(); buildPresets(); };
  showMenu(x, y, [
    { label: "Uygula", fn: async () => { dsp = await invoke("apply_custom", { index: i }); paintAll(); } },
    { label: "Şu anki sesle güncelle", hint: "pedallardaki ayarları bu moda yazar", fn: async () => {
      if (!(await ask({ title: `"${c.name}" güncellensin mi?`, text: "Moddaki eski ayarların yerine pedalların şu anki ayarları yazılacak.", ok: "Güncelle" }))) return;
      try { done(await invoke("update_custom", { index: i })); showBanner(`"${c.name}" güncellendi.`, "ok", 2500); } catch (e) { showBanner(String(e), "warn", 4000); }
    } },
    { label: "Yeniden adlandır", fn: async () => {
      const name = await askText({ title: "Modu yeniden adlandır", value: c.name, ok: "Kaydet" });
      if (!name || name === c.name) return;
      try { done(await invoke("rename_custom", { index: i, name })); } catch (e) { showBanner(String(e), "warn", 4000); }
    } },
    "-",
    { label: "Sil", danger: true, fn: async () => {
      if (!(await ask({ title: `"${c.name}" silinsin mi?`, text: "Bu işlem geri alınamaz. Şu an duyduğun ses değişmez, sadece kayıt silinir.", ok: "Sil", danger: true }))) return;
      done(await invoke("delete_custom", { index: i }));
    } },
  ]);
}

function openSaveForm() {
  ptab = "mine";
  buildPresets();
  $("#save-form").hidden = false;
  const inp = $("#save-name");
  const cur = customPresets[dsp.preset - CUSTOM_BASE];
  inp.value = cur ? cur.name : "";
  inp.focus();
  inp.select();
}

function bindPresetSave() {
  $("#btn-save-preset").addEventListener("click", openSaveForm);
  $("#save-cancel").addEventListener("click", () => ($("#save-form").hidden = true));
  $("#save-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const name = $("#save-name").value.trim();
    if (!name) return $("#save-name").focus();
    try {
      const r = await invoke("save_custom", { name });
      customPresets = r.custom;
      dsp = r.dsp;
      $("#save-form").hidden = true;
      paintAll();
      buildPresets();
    } catch (err) {
      showBanner(String(err), "warn", 5000);
    }
  });
}

// ------------------------------------------------------------------ sampler
function buildPads() {
  const box = $("#pads");
  box.innerHTML = "";
  for (let i = 0; i < 8; i++) {
    const info = pads[i];
    const b = el("button", "pad" + (info ? "" : " empty"));
    b.innerHTML = info ? `<b>${escapeHtml(info.name)}</b><small>${i + 1}</small>` : `<b aria-hidden="true">+</b><small>${i + 1}</small>`;
    b.setAttribute("aria-label", info ? `${info.name} çal` : `Pad ${i + 1} için ses dosyası seç`);
    b.addEventListener("click", () => (pads[i] ? playPad(i) : pickPad(i)));
    b.addEventListener("contextmenu", (e) => { e.preventDefault(); padMenu(i, e.clientX, e.clientY); });
    box.appendChild(b);
  }
}
async function pickPad(i) {
  if (!T) return;
  const path = await T.dialog.open({ multiple: false, filters: [{ name: "Ses", extensions: ["wav", "mp3", "ogg", "flac", "m4a", "aac", "aif", "aiff", "caf"] }] });
  if (!path) return;
  try { pads[i] = await invoke("assign_pad", { index: i, path }); buildPads(); }
  catch (e) { showBanner(`Ses yüklenemedi: ${e}`, "warn", 6000); }
}
function padMenu(i, x, y) {
  if (!pads[i]) return pickPad(i);
  const name = pads[i].name;
  showMenu(x, y, [
    { label: "Çal", hint: `Ctrl+Alt+${i + 1}`, fn: () => playPad(i) },
    { label: "Ses dosyasını değiştir…", fn: () => pickPad(i) },
    { label: "Tümünü durdur", hint: "Ctrl+Alt+0", fn: () => invoke("stop_pads") },
    "-",
    { label: "Pad'i temizle", danger: true, fn: async () => {
      if (!(await ask({ title: `"${name}" kaldırılsın mı?`, text: "Pad boşalır; ses dosyan bilgisayarında kalır.", ok: "Kaldır", danger: true }))) return;
      await invoke("clear_pad", { index: i }); pads[i] = null; buildPads();
    } },
  ]);
}
function playPad(i) { invoke("play_pad", { index: i }); flashPad(i); }
function flashPad(i) {
  const b = $("#pads").children[i];
  if (!b) return;
  b.classList.add("hit");
  setTimeout(() => b.classList.remove("hit"), 180);
}

// ------------------------------------------------------------------ test tezgâhı
function paintBench() {
  $("#btn-monitor").setAttribute("aria-pressed", !!dsp.monitor_on);
  $("#mon-label").textContent = dsp.monitor_on ? "Kulaklıkta dinleniyor" : "Kulaklığa ver";
  $("#ab-out").setAttribute("aria-pressed", dsp.monitor_src !== 1);
  $("#ab-raw").setAttribute("aria-pressed", dsp.monitor_src === 1);
  $("#mon-vol").value = dsp.monitor_db;
  $("#mon-val").textContent = fmt.db(dsp.monitor_db);
}

function bindBench() {
  $("#btn-monitor").addEventListener("click", () => { setDsp({ monitor_on: !dsp.monitor_on }); paintBench(); });
  $("#ab-out").addEventListener("click", () => { setDsp({ monitor_src: 0 }); paintBench(); });
  $("#ab-raw").addEventListener("click", () => { setDsp({ monitor_src: 1 }); paintBench(); });
  $("#mon-vol").addEventListener("input", (e) => { setDsp({ monitor_db: +e.target.value }); paintBench(); });

  $("#btn-rec").addEventListener("click", async () => {
    try { await invoke("test_record", { seconds: 10 }); } catch (e) { showBanner(String(e), "warn"); }
  });
  $("#play-raw").addEventListener("click", () => invoke("test_play", { which: "raw" }));
  $("#play-clean").addEventListener("click", () => invoke("test_play", { which: "clean" }));
  $("#play-stop").addEventListener("click", () => invoke("test_stop"));

  let recTimer = null;
  listen("test", ({ payload: m }) => {
    const btn = $("#btn-rec"), lab = $("#rec-label"), fill = $("#rec-fill"), note = $("#rec-note");
    clearInterval(recTimer);
    if (m.phase === "recording") {
      btn.classList.add("busy"); btn.disabled = true;
      const t0 = Date.now(), dur = m.seconds * 1000;
      fill.style.width = "0";
      note.textContent = "Oyunda konuştuğun gibi konuş.";
      recTimer = setInterval(() => {
        const p = Math.min(1, (Date.now() - t0) / dur);
        fill.style.width = `${p * 100}%`;
        lab.textContent = p < 1 ? `Kaydediliyor… ${Math.ceil((dur - (Date.now() - t0)) / 1000)}` : "İşleniyor…";
      }, 100);
    } else if (m.phase === "processing") {
      lab.textContent = "İşleniyor…";
    } else {
      btn.classList.remove("busy"); btn.disabled = false;
      fill.style.width = "0";
      const ok = m.phase === "ready";
      lab.textContent = ok ? "Tekrar kaydet" : "Kaydet";
      ["#play-raw", "#play-clean", "#play-stop"].forEach((s) => ($(s).disabled = !ok));
      note.textContent = ok ? "Hazır. Önce Ham, sonra Temiz dinle; fark burada duyulur." : `Kayıt başarısız: ${m.error}`;
      if (ok && !m.monitor) note.textContent = "Kulaklık çıkışı bulunamadı; Ayarlar'dan kulaklığını seç.";
    }
  });
}

// ------------------------------------------------------------------ dalga formları (osiloskop)
const wave = { raw: null, out: null };
function drawScope(canvas, pts, color) {
  const dpr = window.devicePixelRatio || 1;
  const W = canvas.clientWidth, H = canvas.clientHeight;
  if (canvas.width !== Math.round(W * dpr)) { canvas.width = Math.round(W * dpr); canvas.height = Math.round(H * dpr); }
  const g = canvas.getContext("2d");
  g.setTransform(dpr, 0, 0, dpr, 0, 0);
  g.clearRect(0, 0, W, H);
  const mid = H / 2;
  if (!pts) { g.strokeStyle = color; g.globalAlpha = .4; g.beginPath(); g.moveTo(0, mid); g.lineTo(W, mid); g.stroke(); g.globalAlpha = 1; return; }
  // sessizlik düz çizgi kalsın, konuşma ekranı doldursun: hafif logaritmik ölçek
  const scale = (v) => Math.sign(v) * Math.min(1, Math.log10(1 + Math.abs(v) * 60) / Math.log10(61));
  g.strokeStyle = color; g.lineWidth = 1.5; g.lineJoin = "round";
  g.beginPath();
  const step = W / (pts.length - 1);
  pts.forEach((v, i) => {
    const y = mid - scale(v) * (mid - 2);
    i ? g.lineTo(i * step, y) : g.moveTo(0, y);
  });
  g.stroke();
}
// Sürekli döngü yerine yalnızca yeni veri gelince, kare başına en fazla bir kez çizilir.
let waveQueued = false;
function drawWaves() {
  if (waveQueued) return;
  waveQueued = true;
  requestAnimationFrame(() => {
    waveQueued = false;
    drawScope($("#wave-raw"), wave.raw, "#FF8A3D");
    drawScope($("#wave-out"), wave.out, "#F2E8D5");
  });
}

// Değeri değişmeyen yazılara dokunma (gereksiz yeniden yerleşim/boyama olmasın)
function setText(e, t) {
  if (e && e.textContent !== t) e.textContent = t;
}

// ------------------------------------------------------------------ metreler
const dbHold = {};
function showDb(sel, db) {
  // tepe değeri kısa süre tutulur ki rakam okunabilsin
  const now = Date.now(), h = dbHold[sel] || { v: -120, t: 0 };
  if (db > h.v || now - h.t > 400) { h.v = db; h.t = now; }
  dbHold[sel] = h;
  const e = $(sel);
  const live = h.v > -70;
  setText(e, live ? `${h.v.toFixed(0)} dB` : "–∞");
  e.classList.toggle("live", live);
}
let needleDb = -60;
let clipSeen = 0, clipWarnUntil = 0;
let lastAng = "";
function onMeters(m) {
  if (document.hidden) return;
  // VU iğnesi: -20..+3 dB → -60..+60 derece
  const target = Math.max(-30, m.out_db);
  needleDb = target > needleDb ? target : needleDb * 0.85 + target * 0.15;
  const ang = Math.max(-62, Math.min(62, ((needleDb + 20) / 23) * 120 - 60)).toFixed(0);
  if (ang !== lastAng) {
    lastAng = ang;
    $("#needle").setAttribute("transform", `rotate(${ang} 75 78)`);
  }

  for (const p of pedalEls) {
    if (p.def.vad && p.gr) setText(p.gr, dsp.ai_on ? (m.vad > 0.6 ? "SES" : "—") : "");
    if (p.def.gr && p.gr) setText(p.gr, dsp[p.def.on] && m.gr_db > 0.3 ? `-${m.gr_db.toFixed(1)}` : "");
  }

  // gecikme ≈ RNNoise bloğu (10 ms) + kendini ayarlayan çıkış tamponu
  const lat = 10 + (m.eff_ms || m.buf_ms || 0);
  setText($("#st-lat"), String(Math.round(lat)));
  if (m.cpu != null) setText($("#st-cpu"), `%${(m.cpu * 100).toFixed(1)}`);
  const under = m.underruns - underBase;
  const ue = $("#st-under");
  setText(ue, String(under));
  ue.classList.toggle("bad", under > 0);
  const mu = (m.mon_underruns || 0) - monUnderBase;
  const me = $("#st-mon-under");
  setText(me, String(mu));
  me.classList.toggle("bad", mu > 0);
  me.parentElement.title = `Kulaklık çıkışındaki kesintiler · tampon şu an ${Math.round(m.mon_eff_ms || 0)} ms`;

  if (m.clips > clipSeen) {
    clipSeen = m.clips;
    if (Date.now() > clipWarnUntil) {
      showBanner("Mikrofon sinyali doyuyor (clipping). UM2 üzerindeki GAIN düğmesini biraz kıs.", "warn", 6000);
      clipWarnUntil = Date.now() + 30000;
    }
  }
  if (m.wave_raw) { wave.raw = m.wave_raw; wave.out = m.wave_out; drawWaves(); }
  showDb("#db-raw", m.in_db);
  showDb("#db-out", m.out_db);
}

// ------------------------------------------------------------------ teşhis
function bindDiag() {
  const dlg = $("#dlg-diag"), body = $("#diag-body"), applyBtn = $("#diag-apply");
  let suggest = null, timer = null;
  $("#btn-diag").addEventListener("click", async () => {
    applyBtn.hidden = true;
    body.innerHTML = `<div class="diag-step"><span class="big">Hazırlanıyor…</span></div>`;
    dlg.showModal();
    try { await invoke("diag_run"); } catch (e) { body.innerHTML = `<p class="tip">${escapeHtml(String(e))}</p>`; }
  });
  applyBtn.addEventListener("click", async () => {
    if (suggest) { dsp = await invoke("set_dsp", { patch: suggest }); paintAll(); }
    dlg.close();
  });
  listen("diag", ({ payload: m }) => {
    clearInterval(timer);
    if (m.phase === "silence" || m.phase === "speech") {
      const txt = m.phase === "silence" ? ["1/2 · Sessiz kal", "Ortam gürültüsü ölçülüyor. Konuşma, klavyeye dokunma."]
                                         : ["2/2 · Normal konuş", "Oyunda konuştuğun gibi konuş, bir kez de yüksek sesle gül ya da bağır."];
      body.innerHTML = `<div class="diag-step"><span class="big">${txt[0]}</span><p class="tip">${txt[1]}</p><div class="bar"><i></i></div></div>`;
      const bar = body.querySelector(".bar > i"), t0 = Date.now(), dur = m.seconds * 1000;
      timer = setInterval(() => { bar.style.width = `${Math.min(100, (Date.now() - t0) / dur * 100)}%`; }, 50);
    } else if (m.phase === "done") {
      const r = m.report;
      suggest = r.suggest;
      const tag = { ok: "İYİ", warn: "UYARI", bad: "SORUN" };
      body.innerHTML = `
        <div class="diag-nums">
          <span>Gürültü <b>${r.noise_db.toFixed(0)} dBFS</b></span>
          <span>Konuşma tepe <b>${r.speech_peak_db.toFixed(0)} dBFS</b></span>
          <span>Kırpılma <b>%${r.clip_pct.toFixed(2)}</b></span>
          <span>Vınlama <b>%${r.hum_pct.toFixed(0)}</b></span>
        </div>
        <div class="findings">${r.findings.map((f) => `
          <div class="finding ${f.level}"><span class="tag">${tag[f.level]}</span>
            <div><b>${escapeHtml(f.title)}</b><p>${escapeHtml(f.detail)}</p></div></div>`).join("")}
        </div>
        <p class="tip">Öneriler: ${Object.entries(r.suggest).map(([k, v]) => `${k} = ${v}`).join(", ")}</p>`;
      applyBtn.hidden = false;
    } else if (m.phase === "error") {
      body.innerHTML = `<p class="tip">Teşhis başarısız: ${escapeHtml(m.error)}</p>`;
    }
  });
}

// ------------------------------------------------------------------ ayarlar
const SET_KNOBS = [
  { p: "in_gain_db", l: "GİRİŞ", min: -12, max: 18, step: 0.5, f: fmt.db, def: 0, note: "Yazılım kazancı. Önce UM2 GAIN'i ayarla." },
  { p: "master_db", l: "ÇIKIŞ", min: -24, max: 12, step: 0.5, f: fmt.db, def: 0, note: "Oyuna giden son seviye" },
  { p: "buffer_ms", l: "TAMPON", min: 10, max: 80, step: 5, f: fmt.ms, def: 20, note: "En az bu kadar; çıtırtıda kendini büyütür" },
  { p: "sb_volume_db", l: "SAMPLER", min: -30, max: 6, step: 1, f: fmt.db, def: -6, note: "Soundboard sesi" },
  { p: "duck_db", l: "DUCKING", min: -30, max: 0, step: 1, f: fmt.db, def: -10, note: "Sampler çalarken mikrofon ne kadar kısılır" },
];
let devOrig = null;

function buildSettings() {
  const box = $("#set-knobs");
  box.innerHTML = "";
  for (const k of SET_KNOBS) {
    const mod = el("div", "set-mod");
    mod.appendChild(buildKnob(k));
    mod.appendChild(el("small", null, k.note));
    box.appendChild(mod);
  }
}

function paintSettings() {
  SET_KNOBS.forEach((k) => paintKnob(k.p));
  document.querySelectorAll("#seg-ch button").forEach((b) => b.setAttribute("aria-pressed", String(dsp.channel) === b.dataset.ch));
  document.querySelectorAll(".led-toggle[data-p]").forEach((b) => b.setAttribute("aria-pressed", !!dsp[b.dataset.p]));
}

function devSelection() {
  return { input: $("#sel-in").value || null, output: $("#sel-out").value || null, monitor: $("#sel-mon").value || null };
}
function checkDevDirty() {
  const cur = devSelection();
  const dirty = !!devOrig && (cur.input !== devOrig.input || cur.output !== devOrig.output || cur.monitor !== devOrig.monitor);
  $("#dev-dirty").hidden = !dirty;
  $("#btn-apply-dev").disabled = !dirty;
}

async function openSettings() {
  const dlg = $("#dlg-settings");
  let list = { inputs: [], outputs: [] };
  try { list = await invoke("list_devices"); } catch (e) { console.error(e); }
  const st = await invoke("get_state");
  const s = st.settings, info = st.status.info || {};
  const fill = (sel, items, cur, auto) => {
    sel.innerHTML = "";
    sel.appendChild(new Option(auto, ""));
    items.forEach((n) => sel.appendChild(new Option(n, n)));
    sel.value = cur || "";
  };
  fill($("#sel-in"), list.inputs, s.input_device, `Otomatik · ${info.input || "UM2"}`);
  fill($("#sel-out"), list.outputs.filter((n) => !/16ch/i.test(n)), s.output_device, `Otomatik · ${info.output || "CABLE Input"}`);
  fill($("#sel-mon"), list.outputs.filter((n) => !/cable/i.test(n)), s.monitor_device, `Windows varsayılanı · ${info.monitor || "yok"}`);
  devOrig = devSelection();
  checkDevDirty();
  $("#tg-autostart").setAttribute("aria-pressed", !!st.autostart);
  paintSettings();
  dlg.showModal();
}

function bindSettings() {
  buildSettings();
  $("#btn-settings").addEventListener("click", openSettings);
  const dlg = $("#dlg-settings");
  // dışarı (arka plana) tıklayınca kapan
  dlg.addEventListener("click", (e) => { if (e.target === dlg) dlg.close(); });
  ["#sel-in", "#sel-out", "#sel-mon"].forEach((s) => $(s).addEventListener("change", checkDevDirty));
  document.querySelectorAll("#seg-ch button").forEach((b) =>
    b.addEventListener("click", () => { setDsp({ channel: +b.dataset.ch }); paintSettings(); }));
  document.querySelectorAll(".led-toggle[data-p]").forEach((b) =>
    b.addEventListener("click", () => { setDsp({ [b.dataset.p]: !dsp[b.dataset.p] }); paintSettings(); }));
  $("#tg-autostart").addEventListener("click", async (e) => {
    const b = e.currentTarget;
    const want = b.getAttribute("aria-pressed") !== "true";
    try { b.setAttribute("aria-pressed", await invoke("set_autostart", { enabled: want })); }
    catch (err) { showBanner(`Otomatik başlatma ayarlanamadı: ${err}`, "warn"); }
  });
  $("#btn-apply-dev").addEventListener("click", async () => {
    const btn = $("#btn-apply-dev");
    btn.disabled = true;
    btn.textContent = "Yeniden bağlanıyor…";
    status = await invoke("set_devices", devSelection());
    btn.textContent = "Cihazları uygula";
    devOrig = devSelection();
    checkDevDirty();
    paintStatus();
  });
}

// ------------------------------------------------------------------ durum
let bannerTimer = null;
/**
 * Üst şerit. kind: "" (hata, kalıcı), "warn" (uyarı), "ok" (bilgi).
 * Uyarı ve bilgi ms verilmezse 6 sn sonra kendiliğinden kapanır; hata kalır.
 * Süreli şeritlerde kalan süre alttaki ince çizgiyle gösterilir.
 */
function showBanner(text, kind = "", ms, action) {
  const b = $("#banner");
  if (ms == null) ms = kind ? 6000 : 0;
  b.className = "banner " + kind;
  b.innerHTML = `<span class="banner-text">${escapeHtml(text)}</span>`;
  const acts = el("div", "banner-acts");
  if (action) {
    const btn = el("button", "btn btn-sm", action.label);
    btn.addEventListener("click", action.fn);
    acts.appendChild(btn);
  }
  const x = el("button", "banner-x", "×");
  x.setAttribute("aria-label", "Kapat");
  x.addEventListener("click", () => { b.hidden = true; clearTimeout(bannerTimer); });
  acts.appendChild(x);
  b.appendChild(acts);
  if (ms) {
    const bar = el("i", "banner-timer");
    bar.style.animationDuration = `${ms}ms`;
    b.appendChild(bar);
  }
  b.hidden = false;
  clearTimeout(bannerTimer);
  if (ms) bannerTimer = setTimeout(() => (b.hidden = true), ms);
}
function paintStatus() {
  const info = status.info;
  document.querySelector(".bench").classList.toggle("idle", !info);
  if (!info) { wave.raw = null; wave.out = null; }
  $("#chip-in").textContent = info ? `${info.input} · ${(info.in_rate / 1000).toFixed(1)}k` : "Mikrofon yok";
  $("#chip-out").textContent = info ? info.output : "Çıkış yok";
  if (status.error) {
    showBanner(status.error, "", 0, { label: "Yeniden dene", fn: async () => { status = await invoke("restart_engine"); paintStatus(); } });
  } else if (!$("#banner").classList.contains("warn") && !$("#banner").classList.contains("ok")) {
    $("#banner").hidden = true;
  }
}

function paintAll() {
  paintPedals();
  if ($("#dlg-settings").open) paintSettings();
  paintPresets();
  paintBench();
  const m = $("#btn-mute");
  m.setAttribute("aria-pressed", !!dsp.mute);
  m.title = dsp.mute ? "Mikrofon susturuldu (Ctrl+Alt+M)" : "Sustur (Ctrl+Alt+M)";
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

// ------------------------------------------------------------------ başlangıç
async function init() {
  const st = await invoke("get_state");
  dsp = st.settings.dsp;
  presetNames = st.presets;
  customPresets = st.settings.custom_presets || [];
  pads = st.settings.pads;
  status = st.status;
  buildBoard();
  buildPresetTabs();
  buildPresets();
  bindPresetSave();
  buildPads();
  bindBench();
  bindDiag();
  bindSettings();
  $("#btn-mute").addEventListener("click", () => { setDsp({ mute: !dsp.mute }); paintAll(); });
  paintAll();
  paintStatus();

  let first = true;
  listen("meters", ({ payload }) => { if (first) { underBase = payload.underruns; monUnderBase = payload.mon_underruns || 0; clipSeen = payload.clips; first = false; } onMeters(payload); });
  // kısayolla mod değişince ilgili sekmeye geç
  listen("dsp", ({ payload }) => { dsp = payload; ptab = tabOfPreset(); buildPresets(); paintAll(); });
  listen("status", ({ payload }) => { status = payload; paintStatus(); });
  listen("pad", ({ payload }) => flashPad(payload));
  drawWaves();
  // pencere küçültülünce/gizlenince backend metre göndermeyi bıraksın
  document.addEventListener("visibilitychange", () => {
    invoke("set_ui_active", { active: !document.hidden }).catch(() => {});
  });
}
init();

// ------------------------------------------------------------------ tarayıcı önizlemesi için sahte backend
function mockInvoke(cmd, args) {
  const D = mockInvoke.dsp || (mockInvoke.dsp = {
    in_gain_db: 0, channel: 0, hpf_on: true, gate_on: true, gate_thresh_db: -50, gate_release_ms: 150, ai_on: true, ai_strength: 0.9,
    comp_on: true, comp_ratio: 3, comp_makeup_db: 6, eq_on: true, eq_low_db: 0, eq_mid_db: 2, eq_high_db: -1, deess_on: true,
    deess_thresh_db: -30, deess_amount: 0.6, fx_on: true, preset: 0, fx_pitch: 0, fx_tone: 0, rev_on: false, rev_mix: 0.2, rev_size: 0.55,
    dly_on: false, dly_ms: 280, dly_fb: 0.3, lim_on: true, lim_ceiling_db: -1, lim_release_ms: 80, master_db: 0, mute: false,
    monitor_on: false, monitor_src: 0, monitor_db: -6, sb_volume_db: -6, sb_to_monitor: true, duck_on: true, duck_db: -10, buffer_ms: 20 });
  const r = {
    get_state: () => ({ settings: { dsp: D, pads: [{ name: "Alkış" }, { name: "Davul" }, { name: "Bruh" }, null, null, null, null, null] },
      status: { running: true, info: { input: "Mikrofon (BEHRINGER UM2)", output: "CABLE Input (VB-Audio Virtual Cable)", in_rate: 48000, monitor: "Kulaklık" } },
      presets: ["Doğal", "Stüdyo", "Yayıncı", "Podcast", "Robot", "Derin", "Sincap", "Kız sesi", "Çocuk", "Canavar", "Uzaylı", "Hayalet", "Radyo", "Telefon", "Telsiz", "Megafon", "Mağara", "Stadyum", "Su altı", "8-bit"], autostart: false }),
    save_custom: () => { (mockInvoke.custom = mockInvoke.custom || []).push({ name: args.name, dsp: { ...D } }); D.preset = 100 + mockInvoke.custom.length - 1; return { custom: mockInvoke.custom, dsp: D }; },
    apply_custom: () => Object.assign(D, { preset: 100 + args.index }),
    delete_custom: () => { mockInvoke.custom.splice(args.index, 1); return { custom: mockInvoke.custom, dsp: D }; },
    set_dsp: () => Object.assign(D, args.patch),
    set_preset: () => Object.assign(D, { preset: args.index, fx_on: true }),
    list_devices: () => ({ inputs: ["Mikrofon (BEHRINGER UM2)"], outputs: ["CABLE Input (VB-Audio Virtual Cable)", "Kulaklık"] }),
  }[cmd];
  return Promise.resolve(r ? r() : null);
}
function mockListen(name, cb) {
  if (name === "meters") setInterval(() => {
    const t = Date.now() / 400;
    const talk = (i) => (Math.sin(i / 9 + t) > 0.2 ? Math.sin(i * 0.9 + t * 7) * 0.4 * Math.sin(i / 18) : 0);
    const raw = Array.from({ length: 160 }, (_, i) => talk(i) + (Math.random() - 0.5) * 0.06);
    const out = Array.from({ length: 160 }, (_, i) => talk(i) * 1.2);
    cb({ payload: { in_db: -14 + Math.sin(t) * 6, out_db: -10 + Math.sin(t) * 6, gr_db: 2.4, vad: 0.8, cpu: 0.018, buf_ms: 20, underruns: 0, clips: 0, wave_raw: raw, wave_out: out } });
  }, 33);
  return Promise.resolve(() => {});
}
