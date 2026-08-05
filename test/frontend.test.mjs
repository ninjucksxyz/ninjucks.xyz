// Lightweight browser tests for the Ninjucks frontend.
// Serves frontend/ over localhost, drives it with Puppeteer, and mocks window.keplr, window.fetch
// (balances + orderbook) and the broadcast hook — nothing hits a real wallet/RPC.

import http from "node:http";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import puppeteer from "puppeteer";

const HERE = dirname(fileURLToPath(import.meta.url));
const HTML = readFileSync(join(HERE, "..", "frontend", "index.html"), "utf8");

let pass = 0, fail = 0;
const ok = (name, cond, extra = "") => {
  if (cond) { console.log(`  ✓ ${name}`); pass++; }
  else { console.log(`  ✗ ${name}${extra ? " — " + extra : ""}`); fail++; }
};

// order book with real-shaped values: best bid 0.9999 USDC/INJ x604, best ask 1.0001 x191
const BOOK = {
  buys: [{ price: "0.0000000000009999", quantity: "604448400000000000000" }],
  sells: [{ price: "0.0000000000010001", quantity: "191210000000000000000" }],
};

const server = http.createServer((req, res) => { res.writeHead(200, { "content-type": "text/html; charset=utf-8" }); res.end(HTML); });
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const URL = `http://127.0.0.1:${server.address().port}/`;
const browser = await puppeteer.launch({ headless: "new", args: ["--no-sandbox"] });

async function freshPage({ withKeplr = false, withHook = false, balances = { inj: "5000000000000000000" }, book = BOOK } = {}) {
  const page = await browser.newPage();
  await page.evaluateOnNewDocument((flags) => {
    window.__calls = [];
    if (flags.withKeplr) window.keplr = {
      enable: async () => {}, getKey: async () => ({ bech32Address: "inj1testxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", pubKey: new Uint8Array([1, 2, 3]) }),
      signDirect: async () => ({ signed: { bodyBytes: new Uint8Array(), authInfoBytes: new Uint8Array() }, signature: { signature: btoa("sig") } }),
    };
    if (flags.withHook) window.__NINJUCKS_TEST_BROADCAST = async (a) => { window.__calls.push(a); return "MOCKHASH0123456789"; };
    const _fetch = window.fetch.bind(window);
    window.fetch = async (url, opt) => {
      const u = String(url);
      if (u.includes("/cosmos/bank/v1beta1/balances/")) return { ok: true, json: async () => ({ balances: Object.entries(flags.balances).map(([denom, amount]) => ({ denom, amount })) }) };
      if (u.includes("/api/exchange/spot/v2/orderbook/")) return { ok: true, json: async () => ({ orderbook: flags.book }) };
      return _fetch(url, opt);
    };
  }, { withKeplr, withHook, balances, book });
  const errors = []; page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(URL, { waitUntil: "networkidle0" });
  page.__errors = errors;
  return page;
}
const text = (page, sel) => page.$eval(sel, (e) => e.textContent.trim());
const val = (page, sel) => page.$eval(sel, (e) => e.value);
const btnText = (page) => text(page, "#swapBtn");
const waitQuote = (page) => page.waitForFunction(() => window.__ninjucks.quote !== null, { timeout: 5000 });
async function connect(page) { await page.click("#swapBtn"); await page.waitForFunction(() => window.__ninjucks.wallet !== null, { timeout: 5000 }); }

try {
  // 1. Loads; INJ/USDC icons; default INJ->USDC.
  {
    const page = await freshPage();
    ok("title is Ninjucks", (await page.title()).includes("Ninjucks"));
    ok("no page errors on load", page.__errors.length === 0, page.__errors[0]);
    ok("input token icon present", (await page.$("#iconIn img")) !== null);
    ok("default pair INJ -> USDC", (await val(page, "#tokenIn")) === "INJ" && (await val(page, "#tokenOut")) === "USDC");
    await page.close();
  }

  // 2. Pure quote math (walkQuote): 1 INJ -> 0.9999 USDC.
  {
    const page = await freshPage();
    const outBase = await page.evaluate((b) => window.__ninjucks.walkQuote(b, true, "1000000000000000000").out.toString(), BOOK);
    ok("walkQuote 1 INJ -> 999900 USDC base", outBase === "999900", outBase);
    const injOut = await page.evaluate((b) => window.__ninjucks.walkQuote(b, false, "1000000").out.toString(), BOOK);
    ok("walkQuote 1 USDC -> INJ (positive)", BigInt(injOut) > 0n, injOut);
    await page.close();
  }

  // 2b. hINJ token + pair constraints (INJ pairs with USDC & hINJ; USDC only with INJ).
  {
    const page = await freshPage();
    const inOpts = await page.$$eval("#tokenIn option", (os) => os.map((o) => o.value));
    ok("hINJ selectable as input", inOpts.includes("hINJ"), inOpts.join(","));
    const injOuts = await page.$$eval("#tokenOut option", (os) => os.map((o) => o.value));
    ok("INJ output options = USDC + hINJ", injOuts.includes("USDC") && injOuts.includes("hINJ"), injOuts.join(","));
    // pick hINJ out → quote runs, ask_denom is hINJ
    await page.select("#tokenOut", "hINJ");
    await page.waitForFunction(() => window.__ninjucks.quote && window.__ninjucks.quote.expected > 0n, { timeout: 5000 });
    const msg = JSON.parse(await text(page, "#msgPreview"));
    ok("INJ->hINJ ask_denom is hINJ", msg.swap.ask_denom.includes("inj1u5zugw"), msg.swap.ask_denom);
    // switch input to USDC → output restricted to INJ only
    await page.select("#tokenIn", "USDC");
    const usdcOuts = await page.$$eval("#tokenOut option", (os) => os.map((o) => o.value));
    ok("USDC output options = INJ only", usdcOuts.length === 1 && usdcOuts[0] === "INJ", usdcOuts.join(","));
    // switch input to hINJ → output INJ only
    await page.select("#tokenIn", "hINJ");
    const hinjOuts = await page.$$eval("#tokenOut option", (os) => os.map((o) => o.value));
    ok("hINJ output options = INJ only", hinjOuts.length === 1 && hinjOuts[0] === "INJ", hinjOuts.join(","));
    await page.close();
  }

  // 3. Live quote populates expected + min received; default venue hallswap.
  {
    const page = await freshPage();
    await waitQuote(page);
    ok("expected output shown (0.9999)", (await val(page, "#recv")) === "0.9999", await val(page, "#recv"));
    ok("min received at 1% (0.9899 USDC)", (await text(page, "#minRec")).includes("0.9899"), await text(page, "#minRec"));
    const msg = JSON.parse(await text(page, "#msgPreview"));
    ok("venue = hallswap by default", msg.swap.venue === "hallswap");
    ok("ask_denom = USDC", msg.swap.ask_denom.includes("/usdc"));
    ok("minimum_receive = min out base (989901)", msg.swap.minimum_receive === "989901", msg.swap.minimum_receive);
    await page.close();
  }

  // 4. Changing slippage rescales min received (no re-fetch).
  {
    const page = await freshPage();
    await waitQuote(page);
    await page.click('#slipChips button[data-s="2"]');
    ok("2% slippage min (0.9799)", (await text(page, "#minRec")).includes("0.9799"), await text(page, "#minRec"));
    const msg = JSON.parse(await text(page, "#msgPreview"));
    ok("min_receive updates to 2% (979902)", msg.swap.minimum_receive === "979902", msg.swap.minimum_receive);
    await page.close();
  }

  // 5. Connect → balances + Swap enabled.
  {
    const page = await freshPage({ withKeplr: true, balances: { inj: "5000000000000000000" } });
    await waitQuote(page); await connect(page);
    ok("balance shown (5 INJ)", (await text(page, "#payBal")).includes("5 INJ"));
    ok("button = Swap", (await btnText(page)) === "Swap" && !(await page.$eval("#swapBtn", (e) => e.disabled)));
    await page.close();
  }

  // 6. Insufficient balance gates.
  {
    const page = await freshPage({ withKeplr: true, balances: { inj: "500000000000000000" } });
    await waitQuote(page); await connect(page);
    ok("insufficient → disabled", await page.$eval("#swapBtn", (e) => e.disabled));
    ok("insufficient label", (await btnText(page)).includes("Insufficient INJ"), await btnText(page));
    await page.close();
  }

  // 7. Full swap uses the quoted min_receive.
  {
    const page = await freshPage({ withKeplr: true, withHook: true, balances: { inj: "5000000000000000000" } });
    await waitQuote(page); await connect(page);
    await page.click("#swapBtn");
    await page.waitForFunction(() => document.querySelector("#status")?.classList.contains("ok"), { timeout: 5000 });
    ok("swap success + link", (await text(page, "#status")).includes("MOCKHASH012345") && (await page.$("#status a")) !== null);
    const c = (await page.evaluate(() => window.__calls))[0];
    ok("amount = 1 INJ base", c?.amount === "1000000000000000000", c?.amount);
    ok("offer denom inj", c?.offerDenom === "inj");
    ok("execMsg carries quoted min_receive", c?.execMsg?.swap?.minimum_receive === "989901", c?.execMsg?.swap?.minimum_receive);
    await page.close();
  }

  // 8. Flip reverses the pair (USDC -> INJ) and re-quotes.
  {
    const page = await freshPage();
    await waitQuote(page);
    await page.click("#flip");
    ok("flipped to USDC -> INJ", (await val(page, "#tokenIn")) === "USDC" && (await val(page, "#tokenOut")) === "INJ");
    await page.waitForFunction(() => window.__ninjucks.quote && window.__ninjucks.quote.expected > 0n, { timeout: 5000 });
    ok("re-quoted INJ out > 0", (await val(page, "#recv")) !== "—");
    await page.close();
  }

  // 9. No liquidity → button disabled with "No liquidity".
  {
    const page = await freshPage({ withKeplr: true, book: { buys: [], sells: [] } });
    await connect(page);
    await page.waitForFunction(() => window.__ninjucks.quote === null || window.__ninjucks.quote.expected === 0n, { timeout: 5000 });
    ok("no-liquidity disables swap", await page.$eval("#swapBtn", (e) => e.disabled));
    ok("no-liquidity label", (await btnText(page)).includes("No liquidity"), await btnText(page));
    await page.close();
  }

  // 10. No Keplr → clear error.
  {
    const page = await freshPage({ withKeplr: false });
    await page.click("#swapBtn");
    await page.waitForFunction(() => document.querySelector("#status")?.classList.contains("err"), { timeout: 5000 });
    ok("no-keplr surfaces error", (await text(page, "#status")).toLowerCase().includes("keplr not found"));
    await page.close();
  }

  // 11. Mainnet gated on governance.
  {
    const page = await freshPage();
    await page.click('#seg button[data-net="mainnet"]');
    ok("mainnet: swap disabled", await page.$eval("#swapBtn", (e) => e.disabled));
    ok("mainnet: governance label", (await btnText(page)).toLowerCase().includes("governance"));
    await page.close();
  }
} finally { await browser.close(); server.close(); }

console.log(`\n${fail === 0 ? "✅" : "❌"} ${pass} passed, ${fail} failed`);
process.exit(fail === 0 ? 0 : 1);
