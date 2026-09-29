import { readFileSync, writeFileSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { join, dirname } from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const require = createRequire(join(root, '.tools/icon-tools/package.json'));
const { optimize } = require('svgo');
const { Resvg } = require('@resvg/resvg-js');
const path = join(root, 'assets/icon.ico');
const old = readFileSync(path);
const sizes = Array.from({ length: old.readUInt16LE(4) }, (_, i) => old[6 + i * 16] || 256);
const source = join(root, 'assets/icon.svg');
const svg = optimize(readFileSync(source, 'utf8'), {
    multipass: true,
    floatPrecision: 5,
    plugins: ['preset-default', 'removeDimensions', 'removeTitle', 'removeDesc',
        { name: 'removeAttrs', params: { attrs: ['aria-labelledby', 'role'] } }],
}).data;
mkdirSync(join(root, '.tools'), { recursive: true });
const temp = mkdtempSync(join(root, '.tools/icon-'));
try {
    const pngs = sizes.map(size => {
        const png = join(temp, `${size}.png`);
        writeFileSync(png, new Resvg(svg, { fitTo: { mode: 'width', value: size } }).render().asPng());
        execFileSync(process.env.OXIPNG || 'oxipng', ['-o', 'max', '-Z', '--threads', '2', '--strip', 'safe', png], { stdio: 'inherit' });
        return readFileSync(png);
    });
    const header = Buffer.alloc(6 + sizes.length * 16);
    header.writeUInt16LE(1, 2);
    header.writeUInt16LE(sizes.length, 4);
    let offset = header.length;
    sizes.forEach((size, i) => {
        const p = 6 + i * 16;
        header[p] = header[p + 1] = size % 256;
        header.writeUInt16LE(1, p + 4);
        header.writeUInt16LE(32, p + 6);
        header.writeUInt32LE(pngs[i].length, p + 8);
        header.writeUInt32LE(offset, p + 12);
        offset += pngs[i].length;
    });
    writeFileSync(source, svg);
    writeFileSync(path, Buffer.concat([header, ...pngs]));
    console.log(`ICO: ${old.length} -> ${offset} bytes`);
} finally {
    if (dirname(temp) !== join(root, '.tools')) throw new Error('Unexpected temporary directory');
    rmSync(temp, { recursive: true });
}
