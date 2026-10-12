"use strict";

// Keep release parsing independent of the page so it can be checked offline.
(() => {
  const latestUrl = "https://api.gitcode.com/api/v5/repos/CrantAI/CrantPeek/releases/latest?type=latest";
  const filenames = {
    macos: "Crant-Peek-macOS.zip",
    windows: "Crant-Peek-Windows.zip",
    installer: "Crant-Peek-Windows-Setup.exe",
  };
  function latestDownloadUrl(platform) {
    if (!Object.prototype.hasOwnProperty.call(filenames, platform)) throw new Error("Unknown download platform");
    return `https://api.gitcode.com/api/v5/repos/CrantAI/CrantPeek/releases/latest/attach_files/${encodeURIComponent(filenames[platform])}/download`;
  }
  function downloadUrl(raw) {
    const url = new URL(raw);
    const domains = ["gitcode.com", "gitcode.net", "atomgit.com", "myhuaweicloud.com"];
    if (url.protocol !== "https:" || url.username || url.password ||
        url.searchParams.has("access_token") ||
        !domains.some(domain => url.hostname === domain || url.hostname.endsWith("." + domain))) {
      throw new Error("Release attachment is not a public GitCode download");
    }
    return url.href;
  }
  function resolveRelease(body) {
    if (!body || body.prerelease === true || body.draft === true || body.release_status === "pre" ||
        !/^v?\d+\.\d+\.\d+$/.test(body.tag_name) || !Array.isArray(body.assets)) {
      throw new Error("No stable release is available");
    }
    const downloads = {};
    for (const [platform, name] of Object.entries(filenames)) {
      const asset = body.assets.find(asset => asset.name === name);
      // Do not confuse source archives with executable packages.
      if (asset?.browser_download_url) {
        try {
          downloadUrl(asset.browser_download_url);
          // Resolve at download time, including when a bookmarked link or an
          // already-open page outlives the current version's signed URL.
          downloads[platform] = latestDownloadUrl(platform);
        }
        catch { /* Leave this platform unavailable until it has a verified download URL. */ }
      }
    }
    if (!Object.keys(downloads).length) throw new Error("Release packages have not been mirrored yet");
    return { version: body.tag_name, downloads };
  }
  const api = { latestUrl, filenames, latestDownloadUrl, resolveRelease };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else globalThis.CrantReleases = api;
})();
