// One SVG defines the website, tray, Dock and Windows application icons.
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
  const application = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" color="#18181b" fill="none"><rect x="88" y="88" width="848" height="848" rx="190" fill="#fafafa" stroke="#dedee3" stroke-width="10"/><g transform="translate(205 205) scale(25.6)">${inner}</g></svg>`;
  const sizes = [16, 24, 32, 48, 64, 128, 256, 512, 1024];
  const bitmaps = new Map(await Promise.all(sizes.map(async size => [size, await png(application, size).toBuffer()])));
  // ICO entries contain PNG images, including 256 px for Explorer's large view.
  const icoHeader = Buffer.alloc(6 + 16 * 7);
  icoHeader.writeUInt16LE(1, 2);
  icoHeader.writeUInt16LE(7, 4);
  let offset = icoHeader.length;
  sizes.slice(0, 7).forEach((size, i) => {
    const entry = 6 + i * 16;
    icoHeader[entry] = icoHeader[entry + 1] = size === 256 ? 0 : size;
    icoHeader.writeUInt16LE(1, entry + 4);
    icoHeader.writeUInt16LE(32, entry + 6);
    icoHeader.writeUInt32LE(bitmaps.get(size).length, entry + 8);
    icoHeader.writeUInt32LE(offset, entry + 12);
    offset += bitmaps.get(size).length;
  });
  const ico = Buffer.concat([icoHeader, ...sizes.slice(0, 7).map(size => bitmaps.get(size))]);
  const chunks = [['icp4', 16], ['icp5', 32], ['icp6', 64], ['ic07', 128], ['ic08', 256], ['ic09', 512], ['ic10', 1024], ['ic11', 32], ['ic12', 64], ['ic13', 256], ['ic14', 512]].map(([type, size]) => {
    const header = Buffer.alloc(8);
    header.write(type);
    header.writeUInt32BE(8 + bitmaps.get(size).length, 4);
    return Buffer.concat([header, bitmaps.get(size)]);
  });
  const icnsHeader = Buffer.alloc(8);
  icnsHeader.write('icns');
  icnsHeader.writeUInt32BE(8 + chunks.reduce((sum, chunk) => sum + chunk.length, 0), 4);
  await Promise.all([
    fs.writeFile(path.join(root, "website/public/assets/logo.svg"), source),
    fs.writeFile(path.join(root, "website/public/assets/favicon.svg"), favicon),
    png(source, 48).toFile(path.join(root, "assets/tray-query.png")),
    png(windows, 48).toFile(path.join(root, "assets/tray-query-windows.png")),
    fs.writeFile(path.join(root, "assets/app-icon.svg"), application + '\n'),
    fs.writeFile(path.join(root, "assets/app-icon.png"), bitmaps.get(1024)),
    fs.writeFile(path.join(root, "assets/app-icon.ico"), ico),
    fs.writeFile(path.join(root, "assets/app-icon.icns"), Buffer.concat([icnsHeader, ...chunks])),
  ]);
  // Native macOS icon: let the system supply the enclosure/material rather
  // than baking a rounded square and glass highlights into a single bitmap.
  const iconDir = path.join(root, "assets/CrantPeek.icon");
  await fs.mkdir(path.join(iconDir, "Assets"), {recursive: true});
  const paths = [...inner.matchAll(/<path\b[^>]*\/>/g)].map(match => match[0]);
  if (paths.length !== 4) throw new Error("Expected the lens, handle, text and sparkle paths");
  const artwork = [paths.slice(0, 2).join(''), paths[2], paths[3]];
  const names = ['Lens', 'Text', 'Sparkle'];
  await Promise.all(names.map((name, index) => {
    const layer = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" fill="none" color="#18181b"><g transform="translate(205 205) scale(25.6)" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">${artwork[index]}</g></svg>`;
    return png(layer, 1024).toFile(path.join(iconDir, "Assets", `${name}.png`));
  }));
  const fillVariants = [
    {value: {solid: "srgb:0.09412,0.09412,0.10588,1.00000"}},
    {appearance: "dark", value: {solid: "srgb:0.95686,0.95686,0.96863,1.00000"}},
    {appearance: "tinted", value: {solid: "srgb:1.00000,1.00000,1.00000,1.00000"}},
  ];
  await fs.writeFile(path.join(iconDir, "icon.json"), JSON.stringify({
    "color-space-for-untagged-svg-colors": "srgb",
    fill: {"automatic-gradient": "srgb:0.96078,0.96078,0.97647,1.00000"},
    "fill-specializations": [
      {value: {"automatic-gradient": "srgb:0.96078,0.96078,0.97647,1.00000"}},
      {appearance: "dark", value: "system-dark"},
    ],
    groups: names.map((name, index) => ({
      name,
      layers: [{name, "image-name": `${name}.png`, "fill-specializations": fillVariants, glass: index !== 1}],
      lighting: "individual",
      "blur-material": 0,
      shadow: {kind: "neutral", opacity: index === 1 ? 0 : 0.18},
      specular: index !== 1,
      translucency: {enabled: index !== 1, value: index === 2 ? 0.15 : 0.08},
    })),
    "supported-platforms": {circles: ["watchOS"], squares: "shared"},
  }, null, 2) + '\n');
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
