// One SVG defines the website, favicon and both platform tray assets.
// Requires sharp: npm install --no-save sharp, then node tools/render-brand-icons.cjs.
const fs = require("node:fs/promises");
const path = require("node:path");
const sharp = require(process.env.PEEK_SHARP_MODULE || "sharp");
const root = path.resolve(__dirname, "..");

async function main() {
  const source = await fs.readFile(path.join(root, "assets/tray-query.svg"), "utf8");
  const inner = source.replace(/^<svg[^>]*>/, "").replace(/<\/svg>\s*$/, "");
  const favicon = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40" color="#18181b" fill="none"><rect x="1" y="1" width="38" height="38" rx="11" fill="#fff" stroke="#e4e4e7"/><g transform="translate(5 5) scale(1.25)">${inner}</g></svg>\n`;
  const white = source.replace('color="#18181b"', 'color="#fafafa"');
  const windows = white.replace(/(<svg[^>]*>)/, '$1<defs><filter id="tray-shadow" x="-30%" y="-30%" width="160%" height="160%"><feDropShadow dx="0" dy="0" stdDeviation=".35" flood-color="#000" flood-opacity=".85"/></filter></defs>')
    .replace('<g transform=', '<g filter="url(#tray-shadow)" transform=');
  const png = (svg, size) => sharp(Buffer.from(svg), {density: 192}).resize(size, size).png({compressionLevel: 9});
  await Promise.all([
    fs.writeFile(path.join(root, "website/public/assets/logo.svg"), source),
    fs.writeFile(path.join(root, "website/public/assets/favicon.svg"), favicon),
    png(source, 48).toFile(path.join(root, "assets/tray-query.png")),
    png(windows, 48).toFile(path.join(root, "assets/tray-query-windows.png")),
  ]);
  // Large website glyphs and actual-size tray glyphs on light/dark surfaces.
  const layers = await Promise.all([source, white, source, white].map((svg, i) => png(svg, i < 2 ? 64 : 18).toBuffer()));
  const background = Buffer.from('<svg width="320" height="150"><path fill="#f4f4f5" d="M0 0h160v150H0z"/><path fill="#28282b" d="M160 0h160v150H160z"/></svg>');
  await fs.mkdir(path.join(root, "dist"), {recursive: true});
  await sharp(background).composite([
    {input: layers[0], left: 48, top: 20}, {input: layers[1], left: 208, top: 20},
    {input: layers[2], left: 71, top: 109}, {input: layers[3], left: 231, top: 109},
  ]).png().toFile(path.join(root, "dist/logo-preview.png"));
  console.log("Rendered website, favicon and transparent tray icons from one SVG.");
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
