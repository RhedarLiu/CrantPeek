const test = require("node:test");
const assert = require("node:assert/strict");
const {resolveRelease, filenames, latestDownloadUrl} = require("../website/public/releases.js");
const asset = (name, url = `https://gitcode.com/CrantAI/CrantPeek/releases/download/v0.1.1/${name}`) => ({name, browser_download_url:url});

test("selects executable packages, never source archives", () => {
  const result = resolveRelease({tag_name:"v0.1.1", prerelease:false, assets:[asset("source.zip"), ...Object.values(filenames).map(name => asset(name))]});
  assert.equal(result.version,"v0.1.1");
  assert.equal(Object.keys(result.downloads).length,3);
  assert.equal(result.downloads.macos,latestDownloadUrl("macos"));
});
test("download links remain identical when a new formal version replaces the old one", () => {
  const old = resolveRelease({tag_name:"v0.1.0",assets:Object.values(filenames).map(name => asset(name))});
  const next = resolveRelease({tag_name:"v0.2.0",assets:Object.values(filenames).map(name => asset(name))});
  assert.deepEqual(old.downloads,next.downloads);
  for (const link of Object.values(next.downloads)) {
    assert.ok(link.includes("/releases/latest/attach_files/"));
    assert.ok(link.endsWith("/download"));
  }
  assert.throws(() => latestDownloadUrl("__proto__"));
});
test("rejects previews, empty metadata and missing executable attachments", () => {
  for (const body of [{}, {tag_name:"v0.1.1",prerelease:true,assets:[]}, {tag_name:"v0.2.0-beta.1",assets:[]}, {tag_name:"v0.1.1",draft:true,assets:[]}, {tag_name:"v0.1.1",assets:[asset("source.zip")]}]) {
    assert.throws(() => resolveRelease(body));
  }
});
test("never redirects download buttons to GitHub, unsafe URLs or personal tokens", () => {
  for (const url of ["javascript:alert(1)", "http://gitcode.com/file", "https://github.com/release.zip", "https://gitcode.com.evil.test/file", "https://user:password@gitcode.com/file", "https://gitcode.com/file?access_token=private"]) {
    assert.throws(() => resolveRelease({tag_name:"v0.1.1",assets:[asset(filenames.macos,url)]}));
  }
});
test("allows independent platform downloads while other attachments are mirroring", () => {
  const result = resolveRelease({tag_name:"v0.1.1",assets:[asset(filenames.windows)]});
  assert.ok(result.downloads.windows);
  assert.equal(result.downloads.macos,undefined);
});
