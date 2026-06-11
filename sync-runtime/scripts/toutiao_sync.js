import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import process from "node:process";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

let chromium;
let axios;
const scriptDir = path.dirname(fileURLToPath(import.meta.url));

function emit(payload) {
  process.stdout.write(`${JSON.stringify(payload)}\n`);
}

function emitProgress(message, extra = {}) {
  emit({ type: "progress", message, ...extra });
}

function loadBuildInfo() {
  const candidates = [
    path.join(scriptDir, "..", "build-info.json"),
    path.join(process.cwd(), "sync-runtime", "build-info.json"),
    path.join(process.cwd(), "build-info.json"),
  ];

  for (const candidate of candidates) {
    if (!fs.existsSync(candidate)) {
      continue;
    }
    try {
      return JSON.parse(fs.readFileSync(candidate, "utf8"));
    } catch {
      return null;
    }
  }

  return null;
}

function parseArgs() {
  const args = process.argv.slice(2);
  const jobIndex = args.indexOf("--job");
  if (jobIndex === -1 || !args[jobIndex + 1]) {
    throw new Error("缺少 --job 参数");
  }
  return {
    jobPath: path.resolve(args[jobIndex + 1]),
    checkLogin: args.includes("--login-status"),
  };
}

function normalizeJobConfig(rawJob) {
  const dataDir = rawJob.dataDir || rawJob.data_dir;
  const downloadDir = rawJob.downloadDir || rawJob.download_dir;
  const knownRemoteIds = rawJob.knownRemoteIds || rawJob.known_remote_ids || [];
  const cdpPort = rawJob.cdpPort || rawJob.cdp_port || 9222;
  const chromeUserDataDir = rawJob.chromeUserDataDir || rawJob.chrome_user_data_dir;
  const maxItems = Number(rawJob.maxItems || rawJob.max_items || 0) || 0;

  if (!dataDir) {
    throw new Error("任务缺少 dataDir/data_dir");
  }
  if (!downloadDir) {
    throw new Error("任务缺少 downloadDir/download_dir");
  }

  return {
    ...rawJob,
    dataDir,
    downloadDir,
    knownRemoteIds,
    cdpPort,
    chromeUserDataDir,
    maxItems,
  };
}

async function loadDeps() {
  try {
    ({ chromium } = await import("playwright"));
    ({ default: axios } = await import("axios"));
  } catch (error) {
    throw new Error(`缺少依赖。请先执行 npm.cmd install。原始错误: ${error.message}`);
  }
}

function ensureDir(target) {
  if (fs.existsSync(target)) {
    return;
  }
  fs.mkdirSync(target, { recursive: true });
}

async function retryFileOperation(action, attempts = 5, delayMs = 400) {
  let lastError;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      return await action();
    } catch (error) {
      lastError = error;
      const code = error && typeof error === "object" ? error.code : "";
      if (!["EPERM", "EBUSY", "EACCES"].includes(code) || attempt === attempts - 1) {
        throw error;
      }
      await new Promise((resolve) => setTimeout(resolve, delayMs * (attempt + 1)));
    }
  }
  throw lastError;
}

function writeTextFile(filePath, content) {
  return retryFileOperation(async () => {
    fs.writeFileSync(filePath, content, "utf8");
  });
}

function hashText(input) {
  return crypto.createHash("sha1").update(input).digest("hex");
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function probeChromeDebugger() {
  const port = arguments[0] || 9222;
  const probe = spawnSync("curl.exe", ["-s", `http://127.0.0.1:${port}/json/version`], { encoding: "utf8" });
  return probe.stdout?.trim() || "";
}

async function connectChrome(job) {
  const chromePaths = [
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
  ];
  const chromeExe = chromePaths.find((item) => fs.existsSync(item));
  if (!chromeExe) {
    throw new Error("未找到 Chrome");
  }

  const candidatePorts = Array.from(new Set([Number(job.cdpPort) || 9222, 19022, 19222, 19333, 19444, 19555]));
  for (const port of candidatePorts) {
    if (probeChromeDebugger(port)) {
      emitProgress(`复用 Chrome 调试端口 ${port}`);
      return chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    }
  }

  const profileDir =
    job.chromeUserDataDir ||
    path.join(process.env.LOCALAPPDATA || "", "Google", "Chrome", "User Data");
  if (job.chromeUserDataDir) {
    ensureDir(profileDir);
  }

  const launchPort = candidatePorts[0];
  if (!probeChromeDebugger(launchPort)) {
    emitProgress(`启动 Chrome 调试端口 ${launchPort}`);
    const chromeProcess = spawn(
      chromeExe,
      [
        `--remote-debugging-port=${launchPort}`,
        "--remote-debugging-address=127.0.0.1",
        "--remote-allow-origins=*",
        `--user-data-dir=${profileDir}`,
        "--no-first-run",
        "about:blank",
      ],
      { detached: true, windowsHide: false, stdio: "ignore" },
    );
    chromeProcess.unref();
    let connected = false;
    for (let attempt = 0; attempt < 15; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 1000));
      if (probeChromeDebugger(launchPort)) {
        connected = true;
        break;
      }
    }
    if (!connected) {
      throw new Error(
        `Chrome 调试端口 ${launchPort} 未就绪。若 Chrome 已在运行，请先完全退出后重试；或手动用 --remote-debugging-port=${launchPort} 启动 Chrome。`,
      );
    }
  }

  return chromium.connectOverCDP(`http://127.0.0.1:${launchPort}`);
}

function normalizeUrl(url) {
  try {
    const parsed = new URL(url);
    parsed.search = "";
    return parsed.toString();
  } catch {
    return url;
  }
}

function resolveAssetUrl(url, baseUrl) {
  if (typeof url !== "string") {
    return "";
  }
  const trimmed = url.trim();
  if (!trimmed || trimmed.startsWith("blob:") || trimmed.startsWith("data:")) {
    return "";
  }
  try {
    if (trimmed.startsWith("//")) {
      return new URL(`https:${trimmed}`).toString();
    }
    return new URL(trimmed, baseUrl).toString();
  } catch {
    return "";
  }
}

function canonicalRemoteId(item) {
  return String(item.remoteId || normalizeUrl(item.sourceUrl));
}

function stableLocalId(item) {
  return hashText(canonicalRemoteId(item));
}

async function findToutiaoPage(browser) {
  let reusablePage = null;
  for (const context of browser.contexts()) {
    for (const page of context.pages()) {
      const url = page.url();
      if (url.includes("toutiao.com")) {
        return page;
      }
      if (
        !reusablePage &&
        (url === "about:blank" || (!url.startsWith("devtools://") && !url.startsWith("chrome://") && !url.startsWith("chrome-extension://")))
      ) {
        reusablePage = page;
      }
    }
  }
  const context = browser.contexts()[0] ?? (await browser.newContext());
  const page = reusablePage || (await context.newPage());
  await page.goto("https://www.toutiao.com/", { waitUntil: "domcontentloaded" });
  return page;
}

async function detectLoginState(page) {
  return page.evaluate(() => {
    const url = location.href;
    const title = document.title || "";
    const bodyText = (document.body?.innerText || "").replace(/\s+/g, " ");
    const anchors = Array.from(document.querySelectorAll("a[href]")).map((anchor) => ({
      href: anchor.href || "",
      text: (anchor.textContent || anchor.getAttribute("title") || "").replace(/\s+/g, " ").trim(),
    }));
    const hasProfileEntry = anchors.some((item) => item.href.includes("/c/user/token/"));
    const hasSourceEntry = anchors.some((item) => (
      item.href.includes("/c/user/token/") &&
      (item.text.includes("我的收藏") || item.href.includes("tab=fav") || item.href.includes("source=mine_profile"))
    ));
    const visibleLoginButtons = Array.from(document.querySelectorAll("a, button, [role='button'], div, span"))
      .map((element) => {
        const text = (element.textContent || "").replace(/\s+/g, " ").trim();
        const rect = element.getBoundingClientRect();
        return { text, top: rect.top, width: rect.width, height: rect.height };
      })
      .filter((item) => item.top >= 0 && item.top < 260 && item.width > 0 && item.height > 0)
      .some((item) => ["登录", "立即登录"].includes(item.text) || item.text.includes("请登录"));
    const loginUrl = /login|passport|sso/i.test(url);
    const loginText = /请登录|立即登录|手机号登录|验证码登录|扫码登录/.test(bodyText);
    const loginRequired = loginUrl || visibleLoginButtons || loginText;

    return {
      url,
      title,
      isLoggedIn: !loginRequired && (hasSourceEntry || hasProfileEntry),
      loginRequired,
      hasProfileEntry,
      hasSourceEntry,
      bodyPreview: bodyText.slice(0, 180),
    };
  });
}

async function ensureLoggedIn(page, source) {
  let loginState = await detectLoginState(page);
  if (loginState.isLoggedIn && !loginState.loginRequired) {
    return loginState;
  }

  emitProgress("未检测到今日头条登录状态，已打开登录页；请在 Chrome 完成登录，程序会自动继续", {
    pageUrl: loginState.url,
    pageTitle: loginState.title,
  });

  if (!page.url().includes("toutiao.com") || /login|passport|sso/i.test(page.url())) {
    await page.goto("https://www.toutiao.com/", { waitUntil: "domcontentloaded", timeout: 120000 }).catch(() => {});
  }

  const startedAt = Date.now();
  while (Date.now() - startedAt < 180000) {
    await sleep(2500);
    loginState = await detectLoginState(page);
    if (loginState.isLoggedIn && !loginState.loginRequired) {
      emitProgress(`已检测到登录，继续跳转到${source === "likes" ? "喜欢" : "收藏"}列表`, {
        pageUrl: loginState.url,
        pageTitle: loginState.title,
      });
      return loginState;
    }
  }

  throw new Error("未登录：请在调试 Chrome 中完成今日头条登录后重试。登录完成后程序会自动进入收藏/喜欢列表。");
}

async function emitLoginStatus(page, source) {
  if (!page.url().includes("toutiao.com")) {
    await page.goto("https://www.toutiao.com/", { waitUntil: "domcontentloaded", timeout: 120000 }).catch(() => {});
  }
  await page.waitForTimeout(1800);
  let loginState = await detectLoginState(page);
  if (!loginState.isLoggedIn && !loginState.url.includes("toutiao.com")) {
    await page.goto("https://www.toutiao.com/", { waitUntil: "domcontentloaded", timeout: 120000 }).catch(() => {});
    await page.waitForTimeout(1800);
    loginState = await detectLoginState(page);
  }
  const loggedIn = loginState.isLoggedIn && !loginState.loginRequired;
  if (loggedIn) {
    const profile = await collectProfile(page);
    if (profile.name || profile.avatarUrl) {
      emit({ type: "profile", profile });
    }
  }
  emit({
    type: "login_status",
    loggedIn,
    loginRequired: !loggedIn,
    source,
    message: loggedIn
      ? "已登录今日头条，可以同步"
      : "未登录：请在大家的chrome浏览器中登录，登录后点击“刷新”",
    pageUrl: loginState.url,
    pageTitle: loginState.title,
  });
}

async function inspectCurrentPage(page, source) {
  await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
  await page.waitForTimeout(400);
  const info = await page.evaluate((currentSource) => {
    const CONTENT_LINK_PATTERN = /\/(article|video|w)\//;
    const getCardNodes = () => {
      const feed = document.querySelector(".profile-tab-feed");
      if (!feed) return [];
      const cards = Array.from(feed.querySelectorAll("[class*='card-wrapper']"))
        .filter((element) => {
          const className = typeof element.className === "string" ? element.className : "";
          if (!/card-wrapper/.test(className)) return false;
          const parentCard = element.parentElement?.closest("[class*='card-wrapper']");
          return !parentCard || !feed.contains(parentCard);
        });
      return cards.length ? cards : Array.from(feed.children);
    };
    const getCardAnchors = (cards) =>
      cards.flatMap((card) =>
        Array.from(card.querySelectorAll("a[href]"))
          .map((anchor) => ({
            href: anchor.href || "",
            text: (anchor.textContent || anchor.getAttribute("title") || "").replace(/\s+/g, " ").trim(),
          }))
          .filter((item) => item.href.includes("toutiao.com") && CONTENT_LINK_PATTERN.test(item.href)),
      );
    const bodyText = document.body?.innerText || "";
    const title = document.title || "";
    const url = location.href;
    const params = new URLSearchParams(location.search);
    const tabCandidates = Array.from(document.querySelectorAll("a, button, [role='tab'], div, span, li"))
      .map((element) => {
        const text = (element.textContent || "").replace(/\s+/g, " ").trim();
        if (!text || text.length > 12) {
          return null;
        }
        const rect = element.getBoundingClientRect();
        if (rect.top < 0 || rect.top > 420 || rect.width <= 0 || rect.height <= 0) {
          return null;
        }
        if (!["全部", "微头条", "书架", "收藏", "喜欢", "赞过", "点赞"].includes(text)) {
          return null;
        }
        const style = window.getComputedStyle(element);
        const parent = element.parentElement;
        const className = `${element.className || ""} ${parent?.className || ""}`.toLowerCase();
        let score = 0;
        if (element.getAttribute("aria-selected") === "true" || parent?.getAttribute("aria-selected") === "true") score += 8;
        if (/(active|current|selected|on)/.test(className)) score += 6;
        if ((style.fontWeight || "400") >= "500") score += 2;
        if ((style.color || "").includes("245") || (style.color || "").includes("255, 0, 0")) score += 3;
        if ((style.borderBottomColor || "").includes("245") || (style.borderBottomWidth || "0px") !== "0px") score += 2;
        if (element instanceof HTMLAnchorElement && element.href === location.href) score += 8;
        return { text, score, href: element instanceof HTMLAnchorElement ? element.href : "" };
      })
      .filter(Boolean);
    const activeTab = tabCandidates.sort((left, right) => right.score - left.score)[0]?.text || "";
    const cardNodes = getCardNodes();
    const contentAnchors = getCardAnchors(cardNodes);
    const keywords = currentSource === "likes" ? ["喜欢", "赞过", "点赞"] : ["收藏", "我的收藏", "favorite"];
    const matchedKeyword =
      (currentSource === "likes" && ["喜欢", "赞过", "点赞"].includes(activeTab) ? activeTab : "") ||
      (currentSource === "favorites" && activeTab === "收藏" ? activeTab : "") ||
      keywords.find((keyword) => title.includes(keyword) || bodyText.includes(keyword) || url.includes(keyword)) ||
      (currentSource === "likes" && ["digg", "like", "likes"].includes((params.get("tab") || "").toLowerCase())
        ? "tab=digg"
        : "") ||
      (currentSource === "favorites" && ["fav", "favorite", "favorites", "collect", "collection"].includes((params.get("tab") || "").toLowerCase())
        ? "tab=fav"
        : "");

    return {
      url,
      title,
      activeTab,
      bodyPreview: bodyText.replace(/\s+/g, " ").slice(0, 300),
      matchedKeyword: matchedKeyword || "",
      toutiaoAnchorCount: contentAnchors.length,
      contentAnchorCount: contentAnchors.length,
      cardCount: cardNodes.length,
      sampleUrls: contentAnchors.slice(0, 5).map((item) => item.href),
    };
  }, source);

  emitProgress(
    `页面诊断：${source === "likes" ? "喜欢" : "收藏"}，当前标签“${info.activeTab || "未知"}”，标题“${info.title}”，卡片 ${info.cardCount || 0} 个，内容链接 ${info.contentAnchorCount} 个`,
    { pageUrl: info.url, pageTitle: info.title },
  );
  return info;
}

async function switchToSourceTab(page, source) {
  await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
  await page.waitForTimeout(400);
  const labels = source === "likes" ? ["喜欢", "赞过", "点赞"] : ["收藏", "我的收藏"];
  let best = null;

  for (const label of labels) {
    const locator = page.getByText(label, { exact: true });
    const count = Math.min(await locator.count(), 12);
    for (let index = 0; index < count; index += 1) {
      const candidate = locator.nth(index);
      const box = await candidate.boundingBox().catch(() => null);
      if (!box || box.y < 0 || box.y > 420 || box.width <= 10 || box.height <= 10) {
        continue;
      }
      const href = await candidate.evaluate((element) => {
        if (element instanceof HTMLAnchorElement) {
          return element.href || "";
        }
        return element.closest("a")?.href || "";
      }).catch(() => "");
      let score = 0;
      if (label.includes("我的")) score += 4;
      if (href.includes("mine_profile")) score += 8;
      if (/tab=(fav|digg|like)/.test(href)) score += 6;
      score += Math.max(0, 500 - box.y) / 100;
      if (!best || score > best.score) {
        best = { label, locator: candidate, href, score };
      }
    }
  }

  if (!best) {
    return null;
  }

  await best.locator.scrollIntoViewIfNeeded().catch(() => {});
  await best.locator.click({ force: true }).catch(async () => {
    await best.locator.evaluate((element) => {
      element.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
    });
  });
  return { text: best.label, href: best.href };
}

async function findSourceListEntry(page, source) {
  return page.evaluate((currentSource) => {
    const keywords = currentSource === "likes" ? ["喜欢", "赞过", "点赞"] : ["收藏", "我的收藏", "favorite"];
    const anchors = Array.from(document.querySelectorAll("a[href]"));
    const candidates = anchors
      .map((anchor) => {
        const href = anchor.href || "";
        const text = (anchor.textContent || anchor.getAttribute("title") || "").replace(/\s+/g, " ").trim();
        const rect = anchor.getBoundingClientRect();
        const visible = rect.width > 0 && rect.height > 0;
        const keyword = keywords.find((item) => text.includes(item) || href.includes(item));
        const isContentLink = /\/(article|video|w)\//.test(href);
        return {
          href,
          text,
          keyword: keyword || "",
          isContentLink,
        };
      })
      .filter((item) => item.keyword && item.href && item.href.includes("toutiao.com") && !item.isContentLink)
      .map((item) => {
        let score = 0;
        if (item.text === item.keyword) score += 4;
        if (item.text.includes("我的")) score += 2;
        if (item.href.includes("favorite") || item.href.includes("collection")) score += 3;
        if (item.href.includes("source=mine_profile")) score += 8;
        if (item.href.includes("tab=fav") || item.href.includes("tab=digg") || item.href.includes("tab=like")) score += 6;
        if (item.href.includes("user")) score += 1;
        if (item.text.length <= 8) score += 1;
        return { ...item, score };
      })
      .sort((left, right) => right.score - left.score);

    if (candidates[0]) {
      return candidates[0];
    }

    const profileEntry = anchors
      .map((anchor) => {
        const href = anchor.href || "";
        const text = (anchor.textContent || anchor.getAttribute("title") || "").replace(/\s+/g, " ").trim();
        return { href, text };
      })
      .find((item) => item.href.includes("/c/user/token/") && (item.text.includes("个人主页") || item.text.length <= 12));

    if (!profileEntry?.href) {
      return null;
    }

    return {
      href: profileEntry.href,
      text: profileEntry.text || "个人主页",
      keyword: currentSource === "likes" ? "喜欢" : "收藏",
      score: 0,
      fallback: true,
    };
  }, source);
}

function normalizeSourceListUrl(url, source) {
  try {
    const sanitized = String(url).replace("?source=", "&source=");
    const parsed = new URL(sanitized, "https://www.toutiao.com/");
    parsed.searchParams.set("tab", source === "likes" ? "digg" : "fav");
    parsed.searchParams.set("source", "mine_profile");
    return parsed.toString();
  } catch {
    return url;
  }
}

function buildSourceListUrl(baseUrl, source) {
  const parsed = new URL(baseUrl);
  parsed.search = "";
  parsed.searchParams.set("tab", source === "likes" ? "digg" : "fav");
  parsed.searchParams.set("source", "mine_profile");
  return parsed.toString();
}

async function resetAllScrollToTop(page) {
  for (let attempt = 0; attempt < 3; attempt += 1) {
    await page.evaluate(() => {
      window.scrollTo({ top: 0, left: 0, behavior: "instant" });
      const candidates = [
        document.scrollingElement,
        document.documentElement,
        document.body,
        document.querySelector(".main-wrapper"),
        document.querySelector(".main-l"),
        ...Array.from(document.querySelectorAll("div")).filter((element) => {
          const style = window.getComputedStyle(element);
          return (
            element.scrollHeight > element.clientHeight + 80 &&
            /(auto|scroll|overlay)/.test(`${style.overflowY} ${style.overflow}`)
          );
        }),
      ].filter(Boolean);
      for (const element of new Set(candidates)) {
        element.scrollTop = 0;
        element.scrollLeft = 0;
      }
    });
    await page.waitForTimeout(500);
  }
}

function isOnTargetList(info, source) {
  const activeTab = info.activeTab || "";
  if (source === "favorites") {
    return activeTab === "收藏";
  }
  if (["喜欢", "赞过", "点赞"].includes(activeTab)) {
    return true;
  }
  try {
    const parsed = new URL(info.url);
    return ["digg", "like", "likes"].includes((parsed.searchParams.get("tab") || "").toLowerCase());
  } catch {
    return false;
  }
}

function assertPageLooksRight(info, source) {
  if (!info.url.includes("toutiao.com")) {
    throw new Error(`当前标签页不是今日头条页面：${info.url}`);
  }
  if (!isOnTargetList(info, source)) {
    throw new Error(
      `当前仍停在“${info.activeTab || "未知"}”标签，不是“${source === "likes" ? "喜欢" : "收藏"}”列表页；标题：${info.title || "无"}；地址：${info.url}`,
    );
  }
  if (!info.matchedKeyword) {
    throw new Error(
      `当前页面不像“${source === "likes" ? "喜欢" : "收藏"}”列表页。未命中对应关键词；标题：${info.title || "无"}；地址：${info.url}`,
    );
  }
  if (info.contentAnchorCount === 0) {
    throw new Error(
      `当前页面未识别到可同步内容链接。标题：${info.title || "无"}；地址：${info.url}`,
    );
  }
}

async function collectList(page, options = {}) {
  const onDiscovered = typeof options.onDiscovered === "function" ? options.onDiscovered : null;
  await page.bringToFront();
  await resetAllScrollToTop(page);
  await page.waitForTimeout(1200);

  const readVisibleCards = async () =>
    page.evaluate(() => {
      const TITLE_TEXT_BLACKLIST = /^(分享|评论|点赞|收藏|喜欢|更多信息|全部|微头条|书架)$/;
      const CONTENT_LINK_PATTERN = /\/(article|video|w)\//;
      const STYLE_PROPS = [
        "display",
        "box-sizing",
        "position",
        "inset",
        "top",
        "right",
        "bottom",
        "left",
        "float",
        "clear",
        "width",
        "min-width",
        "max-width",
        "height",
        "min-height",
        "max-height",
        "margin",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "padding",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "border",
        "border-top",
        "border-right",
        "border-bottom",
        "border-left",
        "border-radius",
        "box-shadow",
        "overflow",
        "overflow-x",
        "overflow-y",
        "font-family",
        "font-size",
        "font-weight",
        "font-style",
        "line-height",
        "letter-spacing",
        "text-align",
        "text-decoration",
        "text-overflow",
        "white-space",
        "word-break",
        "color",
        "background",
        "background-color",
        "background-image",
        "background-position",
        "background-repeat",
        "background-size",
        "opacity",
        "visibility",
        "object-fit",
        "object-position",
        "vertical-align",
        "align-items",
        "justify-content",
        "gap",
        "flex",
        "flex-direction",
        "flex-wrap",
        "transform",
        "z-index",
        "-webkit-box-orient",
        "-webkit-line-clamp",
      ];
      const inlineComputedStyles = (source) => {
        const clone = source.cloneNode(true);
        const sourceNodes = [source, ...source.querySelectorAll("*")];
        const cloneNodes = [clone, ...clone.querySelectorAll("*")];
        sourceNodes.forEach((node, index) => {
          const target = cloneNodes[index];
          if (!(node instanceof Element) || !(target instanceof Element)) return;
          const computed = window.getComputedStyle(node);
          const cssText = STYLE_PROPS
            .map((prop) => {
              const value = computed.getPropertyValue(prop);
              return value ? `${prop}:${value}` : "";
            })
            .filter(Boolean)
            .join(";");
          if (cssText) {
            target.setAttribute("style", `${target.getAttribute("style") || ""};${cssText}`);
          }
          if (target instanceof HTMLAnchorElement) {
            target.setAttribute("target", "_blank");
            target.setAttribute("rel", "noreferrer");
          }
        });
        clone
          .querySelectorAll(".actions-list-wrapper, .profile-feed-card-tools-actions, .qrcode-panel")
          .forEach((element) => element.remove());
        clone.querySelectorAll("*").forEach((element) => {
          const text = (element.textContent || "").replace(/\s+/g, "").trim();
          if (text === "取消收藏") {
            element.remove();
          }
        });
        return clone.outerHTML;
      };
      const getCardNodes = () => {
        const feed = document.querySelector(".profile-tab-feed");
        if (!feed) return [];
        const cards = Array.from(feed.querySelectorAll("[class*='card-wrapper']"))
          .filter((element) => {
            const className = typeof element.className === "string" ? element.className : "";
            if (!/card-wrapper/.test(className)) return false;
            const parentCard = element.parentElement?.closest("[class*='card-wrapper']");
            return !parentCard || !feed.contains(parentCard);
          });
        return cards.length ? cards : Array.from(feed.children);
      };
      const cardNodes = getCardNodes();
      const items = [];
      for (const card of cardNodes) {
        const rect = card.getBoundingClientRect();
        if (rect.bottom <= 0 || rect.top >= window.innerHeight) continue;
        const scrollTop = window.scrollY || document.documentElement.scrollTop || document.body.scrollTop || 0;
        const anchors = Array.from(card.querySelectorAll("a[href]"));
        const anchor = anchors.find((candidate) => {
          const href = candidate.href || "";
          return href.includes("toutiao.com") && CONTENT_LINK_PATTERN.test(href) && !href.includes("#comment");
        });
        if (!anchor) continue;
        const url = anchor.href;
        const titleCandidates = [
          anchor.getAttribute("title") || "",
          anchor.textContent || "",
          ...anchors.map((candidate) => candidate.getAttribute("title") || ""),
          ...anchors.map((candidate) => candidate.textContent || ""),
          ...Array.from(card.querySelectorAll("h1, h2, h3, h4, strong, p, span")).map((node) => node.textContent || ""),
        ]
          .map((value) => value.replace(/\s+/g, " ").trim())
          .filter((value) => value && value.length >= 6 && !TITLE_TEXT_BLACKLIST.test(value) && !/^\d+\s*(评论|播放|展现)$/.test(value));
        const title =
          titleCandidates.sort((left, right) => right.length - left.length)[0] ||
          "";
        if (!title) continue;
        const summary = card.textContent?.replace(/\s+/g, " ").trim()?.slice(0, 220) || "";
        const coverNode = card.querySelector?.("img, video[poster]");
        const coverUrl =
          coverNode?.getAttribute?.("poster") ||
          coverNode?.currentSrc ||
          coverNode?.getAttribute?.("src") ||
          "";
        items.push({
          remoteId: url.split("/").filter(Boolean).pop() || url,
          absoluteTop: scrollTop + rect.top,
          visualTop: rect.top,
          visualBottom: rect.bottom,
          title,
          summary,
          sourceUrl: url,
          contentType: url.includes("/video/") ? "video" : "article",
          coverUrl,
          listHtml: inlineComputedStyles(card) || card.outerHTML || anchor.outerHTML || "",
        });
      }
      return Array.from(new Map(
        items
          .sort((left, right) => left.absoluteTop - right.absoluteTop || left.visualTop - right.visualTop || left.visualBottom - right.visualBottom)
          .map((item) => [item.sourceUrl, item]),
      ).values());
    });

  const scrollListPage = async () => {
    const hoverPoint = await page.evaluate(() => {
      const main = document.querySelector(".main-l") || document.querySelector(".main-wrapper") || document.body;
      const rect = main.getBoundingClientRect();
      return {
        x: Math.max(20, Math.min(window.innerWidth - 20, rect.left + Math.min(rect.width * 0.55, 520))),
        y: Math.max(120, Math.min(window.innerHeight - 80, rect.top + Math.min(rect.height * 0.65, 520))),
      };
    }).catch(() => ({ x: 520, y: 520 }));
    await page.mouse.move(hoverPoint.x, hoverPoint.y).catch(() => {});
    for (let wheelIndex = 0; wheelIndex < 3; wheelIndex += 1) {
      await page.mouse.wheel(0, 650).catch(() => {});
      await page.waitForTimeout(180);
    }
    return page.evaluate(() => {
      const getCardNodes = (root = document) => {
        const feed = root.querySelector?.(".profile-tab-feed") || (root.matches?.(".profile-tab-feed") ? root : null);
        if (!feed) return [];
        const cards = Array.from(feed.querySelectorAll("[class*='card-wrapper']"))
          .filter((element) => {
            const className = typeof element.className === "string" ? element.className : "";
            if (!/card-wrapper/.test(className)) return false;
            const parentCard = element.parentElement?.closest("[class*='card-wrapper']");
            return !parentCard || !feed.contains(parentCard);
          });
        return cards.length ? cards : Array.from(feed.children);
      };
      const cards = getCardNodes();
      const viewportStep = Math.max(650, Math.floor(window.innerHeight * 0.65));
      const rawContainers = [
        document.scrollingElement,
        document.documentElement,
        document.body,
        document.querySelector(".main-wrapper"),
        document.querySelector(".main-l"),
        ...Array.from(document.querySelectorAll("div")).filter((element) => {
          const style = window.getComputedStyle(element);
          return (
            element.scrollHeight > element.clientHeight + 80 &&
            /(auto|scroll|overlay)/.test(`${style.overflowY} ${style.overflow}`)
          );
        }),
      ].filter(Boolean);
      const scrollContainers = Array.from(new Set(rawContainers));
      document.documentElement.style.scrollBehavior = "auto";
      document.documentElement.style.overflowAnchor = "none";
      document.body.style.scrollBehavior = "auto";
      document.body.style.overflowAnchor = "none";
      const beforeWindowTop = window.scrollY || document.documentElement.scrollTop || document.body.scrollTop || 0;
      const beforeStates = scrollContainers.map((element) => ({
        element,
        top: element.scrollTop || 0,
        max: Math.max(0, (element.scrollHeight || 0) - (element.clientHeight || 0)),
      }));
      const primary = beforeStates
        .map((state) => ({
          ...state,
          cardCount: getCardNodes(state.element).length || 0,
          width: state.element.getBoundingClientRect?.().width || window.innerWidth,
        }))
        .filter((state) => state.max > 0)
        .sort((left, right) => right.cardCount - left.cardCount || right.max - left.max || right.width - left.width)[0]?.element;
      if (primary) {
        primary.style.scrollBehavior = "auto";
        primary.style.overflowAnchor = "none";
        primary.scrollTop = Math.min(
          Math.max(0, (primary.scrollHeight || 0) - (primary.clientHeight || 0)),
          (primary.scrollTop || 0) + viewportStep,
        );
      } else {
        window.scrollBy(0, viewportStep);
      }
      window.scrollBy(0, Math.floor(viewportStep * 0.45));

      const afterWindowTop = window.scrollY || document.documentElement.scrollTop || document.body.scrollTop || 0;
      const afterStates = beforeStates.map((state) => {
        const rect = state.element.getBoundingClientRect?.();
        const afterTop = state.element.scrollTop || 0;
        const scrollHeight = state.element.scrollHeight || 0;
        const clientHeight = state.element.clientHeight || window.innerHeight || 0;
        return {
          beforeTop: state.top,
          afterTop,
          moved: Math.abs(afterTop - state.top),
          maxTop: Math.max(0, scrollHeight - clientHeight),
          scrollHeight,
          clientHeight,
          tagName: state.element.tagName || "window",
          className: typeof state.element.className === "string" ? state.element.className.slice(0, 80) : "",
          width: rect?.width || window.innerWidth,
        };
      });
      const activeState = afterStates
        .sort((left, right) => right.moved - left.moved || right.maxTop - left.maxTop || right.width - left.width)[0] || {
          beforeTop: beforeWindowTop,
          afterTop: afterWindowTop,
          moved: Math.abs(afterWindowTop - beforeWindowTop),
          maxTop: 0,
          scrollHeight: document.documentElement.scrollHeight || document.body.scrollHeight || 0,
          clientHeight: window.innerHeight || 0,
          tagName: "window",
          className: "",
        };
      const pageScrollHeight = Math.max(
        document.documentElement.scrollHeight || 0,
        document.body.scrollHeight || 0,
        activeState.scrollHeight || 0,
      );
      const pageClientHeight = Math.max(window.innerHeight || 0, activeState.clientHeight || 0);
      const pageTop = Math.max(afterWindowTop, activeState.afterTop || 0);
      const isAtBottom = pageTop + pageClientHeight >= pageScrollHeight - 40;
      return {
        beforeTop: Math.max(beforeWindowTop, activeState.beforeTop || 0),
        afterTop: pageTop,
        moved: Math.max(Math.abs(afterWindowTop - beforeWindowTop), activeState.moved || 0),
        scrollHeight: pageScrollHeight,
        clientHeight: pageClientHeight,
        isAtBottom,
        cardCount: cards.length,
        container: `${activeState.tagName}${activeState.className ? `.${activeState.className}` : ""}`,
      };
    });
  };

  const seen = new Map();
  let stableRounds = 0;
  let bottomRounds = 0;
  let lastScrollHeight = 0;

  for (let index = 0; index < 80; index += 1) {
    const batch = await readVisibleCards();
    const beforeSize = seen.size;
    for (const item of batch) {
      if (!seen.has(item.sourceUrl)) {
        const orderedItem = { ...item, listOrder: seen.size + 1 };
        seen.set(item.sourceUrl, orderedItem);
        emitProgress(`发现内容 ${seen.size}: ${orderedItem.title} ${orderedItem.sourceUrl}`, {
          candidates: seen.size,
          pageUrl: orderedItem.sourceUrl,
          pageTitle: orderedItem.title,
        });
        onDiscovered?.(orderedItem, seen.size);
      }
    }
    const scrollInfo = await scrollListPage();
    await page.waitForTimeout(1600);
    const afterScrollBatch = await readVisibleCards();
    for (const item of afterScrollBatch) {
      if (!seen.has(item.sourceUrl)) {
        const orderedItem = { ...item, listOrder: seen.size + 1 };
        seen.set(item.sourceUrl, orderedItem);
        emitProgress(`发现内容 ${seen.size}: ${orderedItem.title} ${orderedItem.sourceUrl}`, {
          candidates: seen.size,
          pageUrl: orderedItem.sourceUrl,
          pageTitle: orderedItem.title,
        });
        onDiscovered?.(orderedItem, seen.size);
      }
    }
    if (seen.size === beforeSize) {
      stableRounds += 1;
    } else {
      stableRounds = 0;
    }
    if (scrollInfo.isAtBottom && scrollInfo.scrollHeight === lastScrollHeight) {
      bottomRounds += 1;
    } else {
      bottomRounds = 0;
    }
    lastScrollHeight = scrollInfo.scrollHeight;
    emitProgress(
      `滚动扫描 ${index + 1}：top ${Math.round(scrollInfo.afterTop)}/${Math.round(scrollInfo.scrollHeight)}，移动 ${Math.round(scrollInfo.moved)}，卡片 ${scrollInfo.cardCount}，累计 ${seen.size}，容器 ${scrollInfo.container}`,
      {
      candidates: seen.size,
      pageUrl: page.url(),
      pageTitle: await page.title().catch(() => ""),
      },
    );
    if (bottomRounds >= 3 && stableRounds >= 2) {
      break;
    }
  }

  const finalBatch = await readVisibleCards();
  for (const item of finalBatch) {
    if (!seen.has(item.sourceUrl)) {
      const orderedItem = { ...item, listOrder: seen.size + 1 };
      seen.set(item.sourceUrl, orderedItem);
      emitProgress(`发现内容 ${seen.size}: ${orderedItem.title} ${orderedItem.sourceUrl}`, {
        candidates: seen.size,
        pageUrl: orderedItem.sourceUrl,
        pageTitle: orderedItem.title,
      });
      onDiscovered?.(orderedItem, seen.size);
    }
  }

  const results = Array.from(seen.values());
  return results;
}

async function collectProfile(page) {
  return page.evaluate(() => {
    const text = (selector) => document.querySelector(selector)?.textContent?.replace(/\s+/g, " ").trim() || "";
    const bodyText = document.body?.innerText || "";
    const avatar =
      document.querySelector("img[src*='avatar']") ||
      Array.from(document.querySelectorAll("img")).find((img) => {
        const rect = img.getBoundingClientRect();
        return rect.width >= 80 && rect.height >= 80 && rect.top < 320;
      });
    const name =
      text("[class*='name']") ||
      Array.from(document.querySelectorAll("h1, h2, strong, span"))
        .map((node) => node.textContent?.trim() || "")
        .find((value) => value && value.length <= 12 && !/^\d/.test(value)) ||
      "";
    const readMetric = (label) => {
      const match = bodyText.match(new RegExp(`([\\d.万]+)\\s*${label}`));
      return match?.[1] || "";
    };
    const bioMatch = bodyText.match(/简介[:：]\s*([^\n\r]+)/);
    return {
      name,
      avatarUrl: avatar?.currentSrc || avatar?.getAttribute?.("src") || null,
      likes: readMetric("获赞"),
      followers: readMetric("粉丝"),
      following: readMetric("关注"),
      bio: bioMatch?.[1]?.replace(/\s*更多信息.*/, "").trim() || "",
      updatedAt: new Date().toISOString(),
    };
  });
}

function buildNoResultError(info, source) {
  const sourceLabel = source === "likes" ? "喜欢" : "收藏";
  const samples = info.sampleUrls.length ? info.sampleUrls.join(" | ") : "无";
  return [
    `当前页面未识别到“${sourceLabel}”内容卡片。`,
    `标题：${info.title || "无"}。`,
    `地址：${info.url}。`,
    `页面关键词命中：${info.matchedKeyword || "无"}。`,
    `今日头条链接数：${info.toutiaoAnchorCount}。`,
    `内容链接数：${info.contentAnchorCount}。`,
    `页面文本预览：${info.bodyPreview || "无"}。`,
    `示例链接：${samples}。`,
    `请先在 Chrome 打开今日头条${sourceLabel}列表页后再试。`,
  ].join(" ");
}

function filterIncrementalCandidates(list, knownRemoteIds) {
  const knownSet = new Set((knownRemoteIds || []).map((item) => String(item)));
  return list.filter((item) => {
    const canonicalId = canonicalRemoteId(item);
    const legacyId = hashText(canonicalId);
    return !knownSet.has(canonicalId) && !knownSet.has(legacyId);
  });
}

function filterDownloadCandidates(list, targetRemoteIds) {
  const targetSet = new Set((targetRemoteIds || []).map((item) => String(item)));
  if (!targetSet.size) {
    return [];
  }
  return list.filter((item) => targetSet.has(canonicalRemoteId(item)) || targetSet.has(hashText(canonicalRemoteId(item))));
}

function listItemToScriptItem(job, item) {
  const coverUrl = resolveAssetUrl(item.coverUrl, item.sourceUrl);
  return {
    remote_id: canonicalRemoteId(item),
    list_order: Number.isFinite(item.listOrder) ? item.listOrder : null,
    list_run_id: job.sessionId || job.session_id || null,
    title: item.title,
    summary: textToSummary(item.summary),
    content_text: "",
    author: "",
    content_type: item.contentType || (item.sourceUrl.includes("/video/") ? "video" : "article"),
    source_url: item.sourceUrl,
    cover_url: coverUrl || null,
    cover_path: null,
    article_path: null,
    video_path: null,
    local_dir: null,
    downloaded: false,
    raw: {
      list: item,
      listOnly: true,
      listOrder: Number.isFinite(item.listOrder) ? item.listOrder : null,
      listRunId: job.sessionId || job.session_id || null,
      source: job.source,
    },
  };
}

async function downloadToFile(url, filePath) {
  const response = await retryFileOperation(
    () =>
      axios.get(url, {
        responseType: "arraybuffer",
        timeout: 120000,
        headers: {
          Referer: "https://www.toutiao.com/",
          "User-Agent":
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome Safari/537.36",
        },
      }),
    3,
    800,
  );
  await retryFileOperation(async () => {
    fs.writeFileSync(filePath, Buffer.from(response.data));
  });
}

function textToSummary(text) {
  return (text || "").replace(/\s+/g, " ").trim().slice(0, 500);
}

function normalizeText(text) {
  return (text || "").replace(/\s+/g, " ").trim();
}

async function collectDetail(page, job, item) {
  await page.goto(item.sourceUrl, { waitUntil: "domcontentloaded", timeout: 120000 });
  await page.waitForTimeout(2500);

  const detail = await page.evaluate(() => {
    const readText = (selector) => document.querySelector(selector)?.textContent?.trim() || "";
    const title = document.querySelector("h1")?.textContent?.trim() || document.title;
    const article = document.querySelector("article");
    const contentText = article?.textContent?.trim() || document.body?.innerText?.trim() || "";
    const summary = contentText.slice(0, 500);
    const author = readText("[class*='author']") || readText("a[href*='user']");
    const videoNode = document.querySelector("video");
    const videoUrl =
      videoNode?.currentSrc ||
      videoNode?.getAttribute("src") ||
      videoNode?.querySelector("source")?.src ||
      videoNode?.querySelector("source")?.getAttribute("src") ||
      "";
    const coverNode =
      document.querySelector("article img") ||
      document.querySelector("video[poster]") ||
      document.querySelector("img");
    return {
      title,
      summary,
      contentText,
      author,
      html: document.documentElement.outerHTML,
      videoUrl,
      coverUrl:
        coverNode?.getAttribute?.("poster") ||
        coverNode?.currentSrc ||
        coverNode?.getAttribute?.("src") ||
        "",
      pageUrl: location.href,
    };
  });

  const itemDir = path.join(job.downloadDir, job.source, stableLocalId(item));
  ensureDir(itemDir);

  const articlePath = path.join(itemDir, "article.html");
  const rawPath = path.join(itemDir, "article.json");
  await writeTextFile(articlePath, detail.html);

  let coverPath = null;
  const resolvedCoverUrl = resolveAssetUrl(detail.coverUrl, detail.pageUrl || item.sourceUrl);
  if (resolvedCoverUrl) {
    const coverExt = path.extname(resolvedCoverUrl.split("?")[0]) || ".jpg";
    coverPath = path.join(itemDir, `cover${coverExt}`);
    try {
      await downloadToFile(resolvedCoverUrl, coverPath);
    } catch {
      coverPath = null;
    }
  }

  await writeTextFile(
    rawPath,
    JSON.stringify(
      {
        fetchedAt: new Date().toISOString(),
        source: job.source,
        sourceUrl: item.sourceUrl,
        ...detail,
        coverPath,
      },
      null,
      2,
    ),
  );

  let videoPath = null;
  let downloaded = true;

  const resolvedVideoUrl = resolveAssetUrl(detail.videoUrl, detail.pageUrl || item.sourceUrl);
  if (resolvedVideoUrl) {
    const ext = path.extname(resolvedVideoUrl.split("?")[0]) || ".mp4";
    videoPath = path.join(itemDir, `video${ext}`);
    await downloadToFile(resolvedVideoUrl, videoPath);
  }

  return {
    remote_id: canonicalRemoteId(item),
    title: detail.title || item.title,
    summary: textToSummary(detail.summary || item.summary),
    content_text: normalizeText(detail.contentText),
    author: detail.author,
    content_type: resolvedVideoUrl ? "video" : "article",
    source_url: item.sourceUrl,
    cover_url: resolvedCoverUrl || null,
    cover_path: coverPath,
    article_path: articlePath,
    video_path: videoPath,
    local_dir: itemDir,
    downloaded,
    raw: {
      list: item,
      detail: {
        author: detail.author,
        coverUrl: resolvedCoverUrl,
        videoUrl: resolvedVideoUrl,
      },
    },
  };
}

async function main() {
  const { jobPath, checkLogin } = parseArgs();
  const job = normalizeJobConfig(JSON.parse(fs.readFileSync(jobPath, "utf8")));
  const mode = String(job.mode || "incremental");
  const isVerifyMode = mode.startsWith("verify");
  const isListMode = mode === "list";
  const isDownloadMode = mode === "download";
  const verifyLimit = Math.max(1, Number(job.maxItems || mode.split(":")[1] || 3) || 3);
  ensureDir(job.downloadDir);
  await loadDeps();
  const buildInfo = loadBuildInfo();

  if (buildInfo?.buildLabel) {
    emitProgress(`版本信息：${buildInfo.buildLabel}`, {
      version: buildInfo.version,
      buildDate: buildInfo.buildDate,
      buildTime: buildInfo.buildTime,
    });
  }

  emitProgress("连接 Chrome");
  const browser = await connectChrome(job);
  const page = await findToutiaoPage(browser);
  if (checkLogin) {
    await emitLoginStatus(page, job.source);
    await browser.close();
    return;
  }
  await ensureLoggedIn(page, job.source);

  let pageInfo = await inspectCurrentPage(page, job.source);
  if (!isOnTargetList(pageInfo, job.source)) {
    const switchedTab = await switchToSourceTab(page, job.source);
    if (switchedTab?.text) {
      emitProgress(`当前停在“${pageInfo.activeTab || "未知"}”，尝试点击顶部“${switchedTab.text}”标签`, {
        pageUrl: pageInfo.url,
        pageTitle: pageInfo.title,
      });
      await page.waitForTimeout(2500);
      pageInfo = await inspectCurrentPage(page, job.source);
    }
  }
  if (!isOnTargetList(pageInfo, job.source)) {
    const entry = await findSourceListEntry(page, job.source);
    const targetUrl = entry?.href
      ? entry.fallback
        ? buildSourceListUrl(entry.href, job.source)
        : normalizeSourceListUrl(entry.href, job.source)
      : buildSourceListUrl(pageInfo.url, job.source);
    emitProgress(
      `当前不在${job.source === "likes" ? "喜欢" : "收藏"}列表页，尝试自动跳转：${entry?.text || targetUrl}`,
      { pageUrl: pageInfo.url, pageTitle: pageInfo.title },
    );
    await page.goto(targetUrl, { waitUntil: "domcontentloaded", timeout: 120000 });
    await page.waitForTimeout(2500);
    const loginState = await detectLoginState(page);
    if (loginState.loginRequired && !loginState.isLoggedIn) {
      await ensureLoggedIn(page, job.source);
      await page.goto(targetUrl, { waitUntil: "domcontentloaded", timeout: 120000 });
      await page.waitForTimeout(2500);
    }
    pageInfo = await inspectCurrentPage(page, job.source);
  }
  assertPageLooksRight(pageInfo, job.source);

  const normalizedListUrl = normalizeSourceListUrl(page.url(), job.source);
  emitProgress(`重新载入${job.source === "likes" ? "喜欢" : "收藏"}列表顶部：${normalizedListUrl}`, {
    pageUrl: pageInfo.url,
    pageTitle: pageInfo.title,
  });
  await page.goto(normalizedListUrl, { waitUntil: "domcontentloaded", timeout: 120000 });
  await page.waitForTimeout(2500);
  await resetAllScrollToTop(page);
  pageInfo = await inspectCurrentPage(page, job.source);
  assertPageLooksRight(pageInfo, job.source);

  const profile = await collectProfile(page);
  if (profile.name || profile.avatarUrl) {
    emit({ type: "profile", profile });
  }

  let streamedListSaved = 0;
  const streamedListUrls = new Set();
  emitProgress(`扫描当前今日头条${job.source === "likes" ? "喜欢" : "收藏"}页面`);
  const list = await collectList(page, {
    onDiscovered: isListMode
      ? (item) => {
          if (streamedListUrls.has(item.sourceUrl)) {
            return;
          }
          streamedListUrls.add(item.sourceUrl);
          streamedListSaved += 1;
          emit({ type: "item", item: listItemToScriptItem(job, item) });
        }
      : null,
  });
  if (!list.length) {
    throw new Error(buildNoResultError(pageInfo, job.source));
  }

  if (mode === "diagnose") {
    emitProgress(`页面诊断通过，当前页面识别到 ${list.length} 条候选内容`, {
      candidates: list.length,
      skipped: 0,
      discovered: list.length,
      saved: 0,
      downloaded: 0,
      pageUrl: pageInfo.url,
      pageTitle: pageInfo.title,
    });
    emit({ type: "done", summary: { candidates: list.length, skipped: 0, discovered: list.length, saved: 0, downloaded: 0 } });
    await browser.close();
    return;
  }

  const candidates = isDownloadMode
    ? filterDownloadCandidates(list, job.knownRemoteIds)
    : filterIncrementalCandidates(list, job.knownRemoteIds);
  const selectedCandidates = isVerifyMode
    ? candidates.slice(0, verifyLimit)
    : isListMode
      ? list
      : candidates;
  const skipped = isListMode ? 0 : list.length - candidates.length;
  emitProgress(`${isDownloadMode ? "待下载" : "识别到"} ${list.length} 条候选内容，跳过 ${skipped} 条，${isListMode ? "刷新列表" : isDownloadMode ? "待下载" : "新增"} ${selectedCandidates.length} 条`, {
    candidates: list.length,
    skipped,
    discovered: selectedCandidates.length,
    saved: 0,
    downloaded: 0,
    pageUrl: pageInfo.url,
    pageTitle: pageInfo.title,
  });

  if (!selectedCandidates.length) {
    emit({ type: "done", summary: { candidates: list.length, skipped, discovered: 0, saved: 0, downloaded: 0 } });
    await browser.close();
    return;
  }

  if (isListMode) {
    emit({
      type: "done",
      summary: { candidates: list.length, skipped, discovered: selectedCandidates.length, saved: streamedListSaved, downloaded: 0 },
    });
    await browser.close();
    return;
  }

  if (isVerifyMode) {
    emitProgress(`验证模式：仅抓取前 ${selectedCandidates.length} 条样本，通过后再执行全量同步`, {
      candidates: list.length,
      skipped,
      discovered: selectedCandidates.length,
      saved: 0,
      downloaded: 0,
      pageUrl: pageInfo.url,
      pageTitle: pageInfo.title,
    });
  }

  let saved = 0;
  let downloaded = 0;
  const detailContext = browser.contexts()[0] ?? (await browser.newContext());
  const detailPage = await detailContext.newPage();

  try {
    for (let index = 0; index < selectedCandidates.length; index += 1) {
      const item = selectedCandidates[index];
      emitProgress(`${isDownloadMode ? "下载内容" : "抓取"} ${index + 1}/${selectedCandidates.length}: ${item.title}`, {
        candidates: list.length,
        skipped,
        discovered: selectedCandidates.length,
        processed: index + 1,
        saved,
        downloaded,
      });
      try {
        const detail = await collectDetail(detailPage, job, item);
        if (detail.downloaded) {
          downloaded += 1;
        }
        saved += 1;
        emit({ type: "item", item: detail });
      } catch (error) {
        emit({
          type: "item_error",
          sourceUrl: item.sourceUrl,
          message: error instanceof Error ? error.message : String(error),
        });
      }
    }
  } finally {
    await detailPage.close().catch(() => {});
  }

  emit({ type: "done", summary: { candidates: list.length, skipped, discovered: selectedCandidates.length, saved, downloaded } });
  await browser.close();
}

main().catch((error) => {
  emit({
    type: "error",
    message: error instanceof Error ? error.message : String(error),
    stack: error instanceof Error ? error.stack : undefined,
  });
  process.exit(1);
});
