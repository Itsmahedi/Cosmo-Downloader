/**
 * Cosmo Downloader v1.2.0 — Content Script (Chrome, Edge, Brave, Opera, Firefox)
 * 
 * Injects a compact, floating cosmic download button on qualified videos
 * with an interactive IDM-style quality/format selector menu.
 * 
 * Strictly suppresses buttons on hover mini-previews, thumbnails, and inactive players.
 */

(() => {
"use strict";

// Guard against multiple injections in SPA navigations, iframes, or re-executions
if (typeof window !== "undefined") {
  if (window.__cosmoLoaded) {
    return;
  }
  window.__cosmoLoaded = true;
  // Strictly prevent injecting inside YouTube subframes/ad-iframes
  if (window !== window.top && (window.location.hostname.includes("youtube.com") || window.location.hostname.includes("youtu.be"))) {
    return;
  }
}

const runtime = typeof browser !== "undefined" ? browser : (typeof chrome !== "undefined" ? chrome : null);

const DEFAULT_QUALITIES = [
  { label: "1080p", format: "1080p", audioOnly: false },
  { label: "720p", format: "720p", audioOnly: false },
  { label: "480p", format: "480p", audioOnly: false },
  { label: "360p", format: "360p", audioOnly: false },
  { divider: true },
  { label: "Audio only", format: "audio", audioOnly: true },
  { label: "Best available", format: "best", audioOnly: false }
];

let currentVideoUrl = "";
let currentVideoId = "";
let currentFormats = null;
let formatsLoading = false;
let formatsError = false;
let formatsRequestId = 0;
let formatFetchTimeout = null;

function getYouTubeVideoId(urlStr = (typeof window !== "undefined" ? window.location.href : "")) {
  try {
    const u = new URL(urlStr);
    const host = u.hostname.toLowerCase();
    if (host === "youtube.com" || host.endsWith(".youtube.com") || host === "youtu.be") {
      if (u.searchParams.has("v")) return u.searchParams.get("v");
      if (host === "youtu.be") {
        const parts = u.pathname.split("/").filter(Boolean);
        if (parts.length > 0) return parts[0];
      }
      for (const p of ["/shorts/", "/embed/", "/v/", "/live/"]) {
        if (u.pathname.startsWith(p)) {
          const parts = u.pathname.slice(p.length).split("/").filter(Boolean);
          if (parts.length > 0) return parts[0];
        }
      }
    }
  } catch (_) {}
  return "";
}

function getCurrentStreamingUrl() {
  if (typeof window === "undefined") return "";
  return normalizeStreamingUrl(window.location.href);
}

function fetchFormatsForVideo(url) {
  if (!url) return;
  const isVideoPage = isYouTubeHost() || url.includes("youtube.com") || url.includes("youtu.be") || url.includes("/watch") || url.includes("/shorts/");
  if (!isVideoPage) {
    return;
  }
  const reqId = ++formatsRequestId;
  formatsLoading = true;
  formatsError = false;

  if (formatFetchTimeout) clearTimeout(formatFetchTimeout);
  formatFetchTimeout = setTimeout(() => {
    if (formatsRequestId === reqId && formatsLoading) {
      formatsLoading = false;
      formatsError = true;
      updateAllOpenMenus();
    }
  }, 8000);

  if (runtime && runtime.runtime && runtime.runtime.sendMessage) {
    try {
      runtime.runtime.sendMessage({ action: "getVideoFormats", url }, (res) => {
        if (formatsRequestId !== reqId) return; // stale/superseded request
        if (formatFetchTimeout) clearTimeout(formatFetchTimeout);
        formatsLoading = false;

        if (res && res.success && res.formats) {
          currentFormats = res.formats;
          formatsError = false;
        } else {
          formatsError = true;
        }
        updateAllOpenMenus();
      });
    } catch (_) {
      if (formatsRequestId === reqId) {
        formatsLoading = false;
        formatsError = true;
        updateAllOpenMenus();
      }
    }
  }
}

function onVideoUrlChanged() {
  const newUrl = getCurrentStreamingUrl();
  const newId = getYouTubeVideoId(newUrl);
  if (newUrl === currentVideoUrl && newId === currentVideoId) {
    return;
  }
  // Cancel old in-flight request
  formatsRequestId++;
  if (formatFetchTimeout) {
    clearTimeout(formatFetchTimeout);
    formatFetchTimeout = null;
  }
  currentVideoUrl = newUrl;
  currentVideoId = newId;
  currentFormats = null;
  formatsLoading = false;
  formatsError = false;

  // Clear rows in all attached menus immediately so old sizes are never shown
  updateAllOpenMenus();

  // If on a video page, start fetching formats for the new video
  if (newId || newUrl.includes("/watch") || newUrl.includes("/shorts/")) {
    fetchFormatsForVideo(newUrl);
  }
}

function updateAllOpenMenus() {
  for (const [videoEl, record] of activeAttachedButtons.entries()) {
    const menu = record.container.querySelector(".cosmo-quality-menu");
    if (menu) {
      renderMenuRows(menu, videoEl);
    }
  }
}

function attachItemClickHandler(item, videoEl, menu) {
  item.addEventListener("click", (e) => {
    e.preventDefault();
    e.stopPropagation();
    e.stopImmediatePropagation();

    const format = item.getAttribute("data-format");
    const qualityOption = item._qualityOption || null;
    triggerDownload(videoEl, format, qualityOption);

    const container = menu.closest(".cosmo-video-container");
    const btn = container ? container.querySelector(".cosmo-video-download-btn") : null;
    const textEl = btn ? btn.querySelector(".cosmo-btn-text") : null;
    const prevText = textEl ? textEl.textContent : "Download";
    if (textEl) textEl.textContent = "Sent to CosmoDownloader";
    if (btn) btn.classList.add("sent");

    if (container) {
      container.classList.remove("open");
      container.classList.remove("flip-up");
      if (btn) btn.setAttribute("aria-expanded", "false");
    }

    setTimeout(() => {
      if (textEl) textEl.textContent = prevText;
      if (btn) btn.classList.remove("sent");
    }, 2000);
  });
}

function renderMenuRows(menu, videoEl) {
  const itemsContainer = menu.querySelector(".cosmo-menu-items");
  if (!itemsContainer) return;
  itemsContainer.innerHTML = "";

  const isLive = currentFormats?.is_live;

  if (currentFormats && Array.isArray(currentFormats.qualities) && currentFormats.qualities.length > 0) {
    // 1. Video resolutions that actually exist
    currentFormats.qualities.forEach((q) => {
      const item = document.createElement("button");
      item.className = "cosmo-quality-item";
      item.type = "button";
      item.setAttribute("role", "option");
      item.setAttribute("data-format", q.format);
      item._qualityOption = q;

      const titleSpan = document.createElement("span");
      titleSpan.className = "quality-name";
      titleSpan.textContent = q.label;

      const metaSpan = document.createElement("span");
      metaSpan.className = "meta";
      metaSpan.textContent = isLive ? "" : (q.size_formatted || "");

      item.appendChild(titleSpan);
      item.appendChild(metaSpan);
      attachItemClickHandler(item, videoEl, menu);
      itemsContainer.appendChild(item);
    });

    // Divider
    const hr = document.createElement("hr");
    itemsContainer.appendChild(hr);

    // 2. Audio only
    if (currentFormats.audio) {
      const q = currentFormats.audio;
      const item = document.createElement("button");
      item.className = "cosmo-quality-item audio";
      item.type = "button";
      item.setAttribute("role", "option");
      item.setAttribute("data-format", "audio");
      item._qualityOption = q;

      const titleSpan = document.createElement("span");
      titleSpan.className = "quality-name";
      titleSpan.textContent = q.label;

      const metaSpan = document.createElement("span");
      metaSpan.className = "meta";
      metaSpan.textContent = isLive ? "" : (q.size_formatted || "");

      item.appendChild(titleSpan);
      item.appendChild(metaSpan);
      attachItemClickHandler(item, videoEl, menu);
      itemsContainer.appendChild(item);
    }

    // 3. Best available / Auto
    if (currentFormats.best) {
      const q = currentFormats.best;
      const item = document.createElement("button");
      item.className = "cosmo-quality-item";
      item.type = "button";
      item.setAttribute("role", "option");
      item.setAttribute("data-format", "best");
      item._qualityOption = q;

      const titleSpan = document.createElement("span");
      titleSpan.className = "quality-name";
      titleSpan.textContent = q.label || "Best available";

      const metaSpan = document.createElement("span");
      metaSpan.className = "meta";
      metaSpan.textContent = isLive ? "" : (q.size_formatted || "");

      item.appendChild(titleSpan);
      item.appendChild(metaSpan);
      attachItemClickHandler(item, videoEl, menu);
      itemsContainer.appendChild(item);
    }
  } else {
    // Formats not yet loaded or error: show default rows
    DEFAULT_QUALITIES.forEach((q) => {
      if (q.divider) {
        const hr = document.createElement("hr");
        itemsContainer.appendChild(hr);
        return;
      }

      const item = document.createElement("button");
      item.className = "cosmo-quality-item" + (q.audioOnly ? " audio" : "");
      item.type = "button";
      item.setAttribute("role", "option");
      item.setAttribute("data-format", q.format);

      const titleSpan = document.createElement("span");
      titleSpan.className = "quality-name";
      titleSpan.textContent = q.label;

      const metaSpan = document.createElement("span");
      if (formatsError) {
        metaSpan.className = "meta";
        metaSpan.textContent = "";
      } else {
        metaSpan.className = "meta loading-placeholder";
        metaSpan.textContent = "";
      }

      item.appendChild(titleSpan);
      item.appendChild(metaSpan);
      attachItemClickHandler(item, videoEl, menu);
      itemsContainer.appendChild(item);
    });

    if (formatsError) {
      const errLine = document.createElement("div");
      errLine.className = "cosmo-sizes-unavailable";
      errLine.textContent = "Sizes unavailable";
      itemsContainer.appendChild(errLine);
    }
  }
}

let siteExcluded = false;
function checkSiteExclusion() {
  if (!runtime || !runtime.storage || !runtime.storage.local) return;
  try {
    runtime.storage.local.get("excludedSitesList", (res) => {
      const host = (window.location.hostname || "").toLowerCase();
      const raw = res && typeof res.excludedSitesList === "string" ? res.excludedSitesList : "";
      const list = raw
        .split(/[\s,;]+/)
        .map((s) => s.trim().toLowerCase().replace(/^https?:\/\//, "").split("/")[0])
        .filter(Boolean);
      siteExcluded = list.some((d) => host === d || host.endsWith("." + d));
      if (siteExcluded) {
        for (const [videoEl, record] of activeAttachedButtons.entries()) {
          record.cleanup();
          activeAttachedButtons.delete(videoEl);
        }
      } else {
        scanVideos();
      }
    });
  } catch (_) {}
}

// Track injected UI elements per video element: Map<HTMLVideoElement, { container: HTMLElement, host: HTMLElement } >
const activeAttachedButtons = new Map();

/**
 * Checks whether the current page is YouTube (exact domain match)
 */
function isYouTubeHost(win = window) {
  const hostname = win.location.hostname.toLowerCase();
  return hostname === "youtube.com" || hostname.endsWith(".youtube.com");
}

/**
 * Determines whether a given video element is an unwanted mini-preview or thumbnail.
 * Exported / exposed for unit testing when in Node/CommonJS environment.
 */
function isMiniPreview(videoEl, win = window) {
  if (!videoEl) return true;

  if (isYouTubeHost(win)) {
    // 1. Definite YouTube preview / thumbnail containers
    if (typeof videoEl.closest === "function" && videoEl.closest(
      "ytd-video-preview, " +
      "ytd-inline-preview-renderer, " +
      "#inline-preview-player, " +
      "ytd-thumbnail, " +
      "ytd-compact-video-renderer, " +
      ".ytd-thumbnail"
    )) {
      return true;
    }

    const pathname = win.location.pathname;
    const isWatch = pathname === "/watch" || pathname.startsWith("/watch");
    const isShorts = pathname === "/shorts" || pathname.startsWith("/shorts");

    if (isWatch) {
      // On /watch: Only attach to the main video player inside #movie_player
      if (typeof videoEl.closest !== "function") return true;
      const moviePlayer = videoEl.closest("#movie_player");
      if (!moviePlayer || moviePlayer.id === "inline-preview-player") {
        return true;
      }
      // If html5-main-video exists in player, videoEl must be it
      if (videoEl.classList && !videoEl.classList.contains("html5-main-video")) {
        const mainVid = moviePlayer.querySelector ? moviePlayer.querySelector("video.html5-main-video") : null;
        if (mainVid && mainVid !== videoEl) {
          return true;
        }
      }
      return false;
    }

    if (isShorts) {
      // On /shorts: Only attach to the active reel video
      if (typeof videoEl.closest !== "function") return true;
      const activeReel = videoEl.closest("ytd-reel-video-renderer[is-active], #shorts-player");
      if (!activeReel) {
        return true;
      }
      return false;
    }

    // On any other YouTube page (home feed, subscriptions, search, channel page):
    // All videos here are hover previews or thumbnails - ignore them!
    return true;
  }

  // Non-YouTube hosts:
  // Lenient fallback size check: ignore if strictly less than 320x180
  try {
    if (typeof videoEl.getBoundingClientRect === "function") {
      const rect = videoEl.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0 && (rect.width < 320 || rect.height < 180)) {
        return true;
      }
    }
  } catch (_) {}

  if (videoEl.clientWidth > 0 && videoEl.clientHeight > 0 && (videoEl.clientWidth < 320 || videoEl.clientHeight < 180)) {
    return true;
  }

  return false;
}

/**
 * Resolves the specific host container for a video element without falling back to global selectors.
 */
function getHostContainer(videoEl, win = window) {
  if (!videoEl) return null;

  if (isYouTubeHost(win)) {
    if (typeof videoEl.closest === "function") {
      const moviePlayer = videoEl.closest("#movie_player");
      if (moviePlayer && moviePlayer.id !== "inline-preview-player") {
        return moviePlayer;
      }

      const shortsReel = videoEl.closest("ytd-reel-video-renderer[is-active], #shorts-player");
      if (shortsReel) {
        return shortsReel;
      }

      const playerContainer = videoEl.closest(".html5-video-player, #ytd-player");
      if (playerContainer) {
        return playerContainer;
      }
    }

    return videoEl.parentElement;
  }

  return videoEl.parentElement || videoEl;
}

function attachCosmoDownloadButton(videoEl) {
  if (!videoEl || isMiniPreview(videoEl)) return;

  const hostContainer = getHostContainer(videoEl);
  if (!hostContainer) return;

  // Prevent duplicate injections inside same host container or on same video
  if (hostContainer.querySelector(".cosmo-video-container") || activeAttachedButtons.has(videoEl)) {
    return;
  }

  // Ensure host container has relative positioning
  try {
    const style = window.getComputedStyle(hostContainer);
    if (style.position === "static") {
      hostContainer.style.position = "relative";
    }
  } catch (_) {}

  // Create Container
  const container = document.createElement("div");
  container.className = "cosmo-video-container";

  // Create Download Button: 28px high, 12px / 500, surface bg, 1px line border, radius 6: logo 16px + "Download" + chevron 14px
  const btn = document.createElement("button");
  btn.className = "cosmo-video-download-btn";
  btn.type = "button";
  btn.setAttribute("aria-label", "Download Video");
  btn.setAttribute("aria-haspopup", "true");
  btn.setAttribute("aria-expanded", "false");

  const logoImg = document.createElement("img");
  logoImg.className = "cosmo-btn-logo";
  logoImg.alt = "Cosmo";
  logoImg.src = runtime && runtime.runtime && runtime.runtime.getURL ? runtime.runtime.getURL("icons/icon-16.png") : "icons/icon-16.png";

  const btnText = document.createElement("span");
  btnText.className = "cosmo-btn-text";
  btnText.textContent = "Download";

  const chevSvg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  chevSvg.setAttribute("class", "cosmo-btn-chevron");
  chevSvg.setAttribute("viewBox", "0 0 24 24");
  chevSvg.setAttribute("width", "14");
  chevSvg.setAttribute("height", "14");
  chevSvg.setAttribute("fill", "none");
  chevSvg.setAttribute("stroke", "currentColor");
  chevSvg.setAttribute("stroke-width", "1.6");
  chevSvg.setAttribute("stroke-linecap", "round");
  chevSvg.setAttribute("stroke-linejoin", "round");
  chevSvg.innerHTML = '<path d="M7 10l5 5 5-5"/>';

  btn.appendChild(logoImg);
  btn.appendChild(btnText);
  btn.appendChild(chevSvg);

  // Create Quality Dropdown Menu: min 176px wide, surface bg, 1px line, radius 10, 4px padding, shadow 0 8px 24px rgba(0,0,0,.18)
  const menu = document.createElement("div");
  menu.className = "cosmo-quality-menu";
  menu.setAttribute("role", "listbox");
  menu.setAttribute("aria-label", "Select quality");

  // Menu Header Row: logo 16px + label, 12px muted 500
  const menuHeader = document.createElement("div");
  menuHeader.className = "cosmo-menu-header";
  const mLogo = document.createElement("img");
  mLogo.src = logoImg.src;
  mLogo.alt = "Cosmo";
  const mLabel = document.createElement("span");
  mLabel.textContent = "Cosmo Downloader";
  menuHeader.appendChild(mLogo);
  menuHeader.appendChild(mLabel);
  menu.appendChild(menuHeader);

  // Dynamic menu items container
  const itemsContainer = document.createElement("div");
  itemsContainer.className = "cosmo-menu-items";
  menu.appendChild(itemsContainer);

  // Initial population of menu rows
  renderMenuRows(menu, videoEl);

  // Menu Footer: 12px muted, centered
  const menuFooter = document.createElement("div");
  menuFooter.className = "cosmo-menu-footer";
  menuFooter.textContent = "© 2026 Mahedi";
  menu.appendChild(menuFooter);

  // Prevent parent player events from interfering or closing the menu
  ["pointerdown", "mousedown", "mouseup", "click"].forEach((evtName) => {
    container.addEventListener(evtName, (e) => {
      e.stopPropagation();
    });
  });

  const openMenu = () => {
    container.classList.add("open");
    btn.setAttribute("aria-expanded", "true");
    renderMenuRows(menu, videoEl);
    if (!currentFormats && !formatsLoading) {
      fetchFormatsForVideo(getCurrentStreamingUrl());
    }
    try {
      const rect = btn.getBoundingClientRect();
      const menuHeight = 240;
      if (rect.bottom + menuHeight > (window.innerHeight || document.documentElement.clientHeight || 900)) {
        container.classList.add("flip-up");
      } else {
        container.classList.remove("flip-up");
      }
    } catch (_) {}
  };

  const closeMenu = () => {
    container.classList.remove("open");
    container.classList.remove("flip-up");
    btn.setAttribute("aria-expanded", "false");
  };

  // Toggle dropdown on button click
  btn.addEventListener("click", (e) => {
    e.preventDefault();
    e.stopPropagation();
    e.stopImmediatePropagation();
    if (container.classList.contains("open")) {
      closeMenu();
    } else {
      openMenu();
    }
  });

  // Close menu on Escape key
  const onKeyDown = (e) => {
    if (e.key === "Escape" && container.classList.contains("open")) {
      e.preventDefault();
      closeMenu();
      btn.focus();
    }
  };
  document.addEventListener("keydown", onKeyDown);

  // Keep button solidly visible during user interaction
  const showBtn = () => {
    btn.style.opacity = "1";
    btn.style.pointerEvents = "auto";
  };

  const handleMouseLeave = () => {
    if (container.classList.contains("open")) return;
    btn.style.opacity = "0.92";
  };

  hostContainer.addEventListener("mouseenter", showBtn);
  hostContainer.addEventListener("mousemove", showBtn);
  hostContainer.addEventListener("mouseleave", handleMouseLeave);
  container.addEventListener("mouseenter", showBtn);
  container.addEventListener("mousemove", showBtn);
  container.addEventListener("mouseleave", handleMouseLeave);
  btn.addEventListener("mouseenter", showBtn);
  btn.addEventListener("mousemove", showBtn);
  menu.addEventListener("mouseenter", showBtn);
  menu.addEventListener("mousemove", showBtn);

  // Close menu only when clicking outside
  const onPointerDownOutside = (e) => {
    if (!container.classList.contains("open")) return;
    const path = typeof e.composedPath === "function" ? e.composedPath() : [];
    if (!container.contains(e.target) && !path.includes(container)) {
      closeMenu();
    }
  };
  document.addEventListener("pointerdown", onPointerDownOutside);

  container.appendChild(btn);
  container.appendChild(menu);
  hostContainer.appendChild(container);

  activeAttachedButtons.set(videoEl, {
    container,
    hostContainer,
    cleanup: () => {
      document.removeEventListener("pointerdown", onPointerDownOutside);
      document.removeEventListener("keydown", onKeyDown);
      hostContainer.removeEventListener("mouseenter", showBtn);
      hostContainer.removeEventListener("mousemove", showBtn);
      hostContainer.removeEventListener("mouseleave", handleMouseLeave);
      container.remove();
    }
  });
}

function cleanTitleFallback(raw) {
  if (!raw) return "";
  let clean = String(raw).trim();
  // Remove leading notification counter e.g. "(19) "
  clean = clean.replace(/^\(\d+\)\s*/, "");
  // Remove trailing " - YouTube" or " | YouTube"
  clean = clean.replace(/\s*[-|]\s*YouTube$/i, "");
  return clean.trim();
}

function extractVideoTitle() {
  // 1. Try window.ytInitialPlayerResponse.videoDetails.title
  try {
    if (window.ytInitialPlayerResponse?.videoDetails?.title) {
      const t = cleanTitleFallback(window.ytInitialPlayerResponse.videoDetails.title);
      if (t) return t;
    }
  } catch (_) {}

  // 1b. Try parsing from script tags containing ytInitialPlayerResponse
  try {
    const scripts = document.querySelectorAll("script");
    for (const s of scripts) {
      const text = s.textContent || "";
      if (text.includes("ytInitialPlayerResponse")) {
        const match = text.match(/ytInitialPlayerResponse\s*=\s*({.+?});/);
        if (match && match[1]) {
          try {
            const data = JSON.parse(match[1]);
            if (data?.videoDetails?.title) {
              const t = cleanTitleFallback(data.videoDetails.title);
              if (t) return t;
            }
          } catch (_) {}
        }
      }
    }
  } catch (_) {}

  // 2. Try meta[property="og:title"]
  try {
    const ogTitle = document.querySelector('meta[property="og:title"]')?.getAttribute("content");
    if (ogTitle) {
      const t = cleanTitleFallback(ogTitle);
      if (t) return t;
    }
  } catch (_) {}

  // 3. Try oEmbed title
  try {
    const oembedLink = document.querySelector('link[type="application/json+oembed"]')?.getAttribute("title");
    if (oembedLink) {
      const t = cleanTitleFallback(oembedLink);
      if (t) return t;
    }
  } catch (_) {}

  // 4. Fallback to document.title cleaned
  const fallback = cleanTitleFallback(document.title);
  return fallback || "video";
}

function sanitizeFilename(name) {
  let clean = cleanTitleFallback(name);
  let res = '';
  let prevSpace = false;

  for (const c of clean) {
    const code = c.codePointAt(0);

    // Never write U+FFFD () into the filename
    if (code === 0xFFFD) {
      continue;
    }

    // Keep emoji joiners/selectors
    const isEmojiModifier = code === 0x200D || code === 0xFE0F;

    // Drop invisible & bidi control characters
    if (!isEmojiModifier && (
      code === 0x200B || code === 0x200C || code === 0x200E || code === 0x200F
      || (code >= 0x202A && code <= 0x202E)
      || (code >= 0x2060 && code <= 0x2064)
      || code === 0xFEFF || code === 0x00AD || code === 0x180E
    )) {
      continue;
    }

    // Replace non-standard spaces with normal space
    const isNonstandardSpace = code === 0x00A0
      || (code >= 0x2000 && code <= 0x200A)
      || code === 0x202F || code === 0x205F || code === 0x3000;

    if (isNonstandardSpace || /\s/.test(c)) {
      if (!prevSpace) {
        res += ' ';
        prevSpace = true;
      }
      continue;
    }

    // Drop ASCII control chars without inserting space
    if (code < 32 && !isEmojiModifier) {
      continue;
    }

    // Strip characters forbidden by Windows: < > : " / \ | ? *
    // Replace with space so adjacent words do not concatenate, collapsing any double spaces.
    if (/[<>:"/\\|?*]/.test(c)) {
      if (!prevSpace) {
        res += ' ';
        prevSpace = true;
      }
      continue;
    }

    res += c;
    prevSpace = false;
  }

  res = res.trim().replace(/\.+$/, '').trim();
  if (!res) return 'download.bin';

  const stem = res.split('.')[0].toUpperCase();
  const reserved = ['CON', 'PRN', 'AUX', 'NUL', 'COM0', 'COM1', 'COM2', 'COM3', 'COM4', 'COM5', 'COM6', 'COM7', 'COM8', 'COM9', 'LPT0', 'LPT1', 'LPT2', 'LPT3', 'LPT4', 'LPT5', 'LPT6', 'LPT7', 'LPT8', 'LPT9'];
  if (reserved.includes(stem)) {
    res = '_' + res;
  }
  return res;
}

function normalizeStreamingUrl(rawUrl) {
  try {
    const u = new URL(rawUrl.trim());
    if (u.searchParams.has('v')) {
      u.searchParams.delete('list');
      u.searchParams.delete('start_radio');
      u.searchParams.delete('index');
      return u.toString();
    }
  } catch (_) {}
  return rawUrl.trim();
}

let lastDownloadTriggered = { url: "", timestamp: 0 };

function triggerDownload(videoEl, format, qualityOption = null) {
  let videoUrl = videoEl ? (videoEl.src || videoEl.currentSrc || "") : "";
  if (!videoUrl && videoEl) {
    const srcEl = videoEl.querySelector ? videoEl.querySelector("source") : null;
    if (srcEl) videoUrl = srcEl.src || "";
  }

  const rawPageUrl = window.location.href;
  const pageUrl = normalizeStreamingUrl(rawPageUrl);
  const isStreaming = pageUrl.includes("youtube.com") || pageUrl.includes("youtu.be") || pageUrl.includes("vimeo.com");
  const urlToSend = (isStreaming || !videoUrl || videoUrl.startsWith("blob:")) ? pageUrl : videoUrl;

  const now = Date.now();
  if (lastDownloadTriggered.url === urlToSend && lastDownloadTriggered.format === format && (now - lastDownloadTriggered.timestamp) < 400) {
    console.log("[CosmoDownloader] Suppressing accidental rapid double click within 400ms for:", urlToSend);
    return;
  }
  lastDownloadTriggered = { url: urlToSend, format: format, timestamp: now };

  const pageTitle = (currentFormats && currentFormats.title) ? currentFormats.title : extractVideoTitle();
  const ext = format === "audio" ? ".mp3" : ".mp4";
  const safeFilename = sanitizeFilename(pageTitle) + ext;

  try {
    runtime.runtime.sendMessage({
      action: "downloadVideo",
      url: urlToSend,
      pageUrl: pageUrl,
      title: pageTitle,
      filename: safeFilename,
      quality: format,
      size: qualityOption?.size || null,
      sizeFormatted: qualityOption?.size_formatted || null,
      isApprox: qualityOption?.is_approx ?? null,
      formatId: qualityOption?.format_id || null,
    });
  } catch (err) {
    console.warn("[CosmoDownloader] Send message failed:", err);
  }
}

/**
 * Scans video elements, cleans up detached or unqualified buttons, and attaches to valid videos.
 */
function scanVideos() {
  if (siteExcluded) return;

  const curUrl = getCurrentStreamingUrl();
  if (curUrl && curUrl !== currentVideoUrl) {
    onVideoUrlChanged();
  }

  // 1. Audit active attachments: remove if video is detached or no longer qualifies
  for (const [videoEl, record] of activeAttachedButtons.entries()) {
    if (!videoEl.isConnected || isMiniPreview(videoEl)) {
      record.cleanup();
      activeAttachedButtons.delete(videoEl);
    }
  }

  // 2. Scan all current video elements
  const videos = document.querySelectorAll("video");
  videos.forEach((videoEl) => {
    if (isMiniPreview(videoEl)) {
      if (activeAttachedButtons.has(videoEl)) {
        activeAttachedButtons.get(videoEl).cleanup();
        activeAttachedButtons.delete(videoEl);
      }
      return;
    }
    if (!activeAttachedButtons.has(videoEl)) {
      attachCosmoDownloadButton(videoEl);
    }
  });
}

// Debounced scan for dynamic mutations and SPA navigation
let scanDebounceTimer = null;
function debouncedScanVideos() {
  if (siteExcluded) return;
  if (scanDebounceTimer) clearTimeout(scanDebounceTimer);
  scanDebounceTimer = setTimeout(scanVideos, 200);
}

// Observe dynamic video additions
if (typeof MutationObserver !== "undefined" && typeof document !== "undefined" && document.documentElement) {
  const observer = new MutationObserver(() => {
    debouncedScanVideos();
  });
  observer.observe(document.documentElement, { childList: true, subtree: true });

  // Initial scan + exclusion check
  checkSiteExclusion();
  currentVideoUrl = getCurrentStreamingUrl();
  currentVideoId = getYouTubeVideoId(currentVideoUrl);
  if (currentVideoId || currentVideoUrl.includes("/watch") || currentVideoUrl.includes("/shorts/")) {
    fetchFormatsForVideo(currentVideoUrl);
  }
  scanVideos();
  setInterval(() => {
    if (!siteExcluded && (isYouTubeHost() || document.querySelector("video") !== null)) {
      scanVideos();
    }
  }, 3000);

  if (runtime && runtime.storage && runtime.storage.onChanged) {
    runtime.storage.onChanged.addListener((changes, area) => {
      if (area === "local" && changes.excludedSitesList) {
        checkSiteExclusion();
      }
    });
  }

  // SPA navigation hooks
  window.addEventListener("yt-navigate-finish", () => {
    onVideoUrlChanged();
    scanVideos();
    setTimeout(scanVideos, 300);
  });
  window.addEventListener("spfdone", () => {
    onVideoUrlChanged();
    scanVideos();
  });
  window.addEventListener("popstate", () => {
    onVideoUrlChanged();
    scanVideos();
  });
  window.addEventListener("hashchange", () => {
    onVideoUrlChanged();
    scanVideos();
  });
}

// --- Direct Link Click Interception (IDM Style) ---
const DOWNLOAD_EXTENSIONS = new Set([
  "zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso", "dmg", "cab",
  "exe", "msi", "msix", "appx", "apk", "deb", "rpm", "appimage", "pkg", "run",
  "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods",
  "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "ts", "3gp",
  "mp3", "flac", "wav", "aac", "ogg", "wma", "m4a", "opus",
  "torrent", "img", "bin", "rom"
]);

function getLinkExtension(urlStr) {
  try {
    const u = new URL(urlStr, window.location.href);
    const pathname = u.pathname;
    const dot = pathname.lastIndexOf(".");
    if (dot !== -1) {
      return pathname.substring(dot + 1).toLowerCase().split(/[?#]/)[0];
    }
  } catch (_) {
    const clean = String(urlStr || "").split(/[?#]/)[0];
    const dot = clean.lastIndexOf(".");
    if (dot !== -1) {
      return clean.substring(dot + 1).toLowerCase();
    }
  }
  return "";
}

if (typeof document !== "undefined") {
  document.addEventListener("click", (e) => {
    if (siteExcluded) return;
    // Holding Alt key bypasses interception (standard IDM shortcut to force native browser download)
    if (e.altKey) return;

    const target = e.target;
    if (!target || typeof target.closest !== "function") return;
    const link = target.closest("a[href]");
    if (!link) return;

    const href = link.href;
    if (!href) return;
    const isMagnet = href.toLowerCase().startsWith("magnet:");
    if (!isMagnet && !/^https?:\/\//i.test(href)) return;

    // Never intercept YouTube internal links (e.g. /watch?v=...)
    if (isYouTubeHost() && (href.includes("/watch?") || href.includes("/shorts/"))) return;

    const hasDownloadAttr = link.hasAttribute("download");
    const ext = getLinkExtension(href);
    const isDownloadable = isMagnet || (ext && DOWNLOAD_EXTENSIONS.has(ext)) || hasDownloadAttr;

    if (isDownloadable) {
      e.preventDefault();
      e.stopPropagation();

      const suggestedName = link.getAttribute("download") || href.split("/").pop()?.split("?")[0] || "download";
      if (runtime && runtime.runtime && runtime.runtime.sendMessage) {
        runtime.runtime.sendMessage({
          action: "downloadLink",
          url: href,
          filename: suggestedName,
          referrer: window.location.href,
        });
      }
    }
  }, true);
}

// Export for Node unit testing if applicable
if (typeof module !== "undefined" && module.exports) {
  module.exports = { isMiniPreview, isYouTubeHost, getHostContainer };
}
})();
