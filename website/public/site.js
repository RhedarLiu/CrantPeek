"use strict";

// Native details keeps download choices keyboard-accessible.
const menu = document.querySelector(".download-menu");
const toggle = menu.querySelector("summary");
const links = [...menu.querySelectorAll(".menu-panel a")];

document.addEventListener("click", (event) => {
  if (event.target.closest('[data-release-asset][aria-disabled="true"]')) event.preventDefault();
  if (!menu.contains(event.target)) menu.open = false;
});
menu.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    menu.open = false;
    toggle.focus();
    event.preventDefault();
  } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    menu.open = true;
    const current = links.indexOf(document.activeElement);
    const next = current < 0
      ? (event.key === "ArrowDown" ? 0 : links.length - 1)
      : (current + (event.key === "ArrowDown" ? 1 : -1) + links.length) % links.length;
    links[next].focus();
  }
});
menu.addEventListener("focusout", () => {
  setTimeout(() => {
    if (!menu.contains(document.activeElement)) menu.open = false;
  }, 0);
});

// Metadata only supplies the version and package availability. Download links
// stay on latest even across future releases, without a frontend account token.
async function refreshDownloads() {
  const state = document.getElementById("release-state");
  const detail = document.getElementById("release-detail");
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 10000);
  try {
    const response = await fetch(CrantReleases.latestUrl, {
      signal: controller.signal, cache: "no-store", credentials: "omit",
    });
    if (!response.ok) throw new Error("Release metadata unavailable");
    const release = CrantReleases.resolveRelease(await response.json());
    let missing = false;
    for (const link of document.querySelectorAll("[data-release-asset]")) {
      const url = release.downloads[link.dataset.releaseAsset];
      link.href = url || "#downloads";
      if (url) link.removeAttribute("aria-disabled");
      else { link.setAttribute("aria-disabled", "true"); missing = true; }
    }
    state.textContent = `最新正式版 ${release.version}${missing ? " · 部分附件同步中" : ""}`;
    detail.textContent = `当前下载：${release.version}。`;
  } catch {
    // Never send users to source-code archives when executable files are unavailable.
    state.textContent = "暂未取得版本信息，下载按钮仍指向最新正式版";
    detail.textContent = "下载链接自动获取最新正式版，当前版本信息暂不可用。";
  } finally {
    clearTimeout(timeout);
  }
}
refreshDownloads();
