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

function dib(pixels, size) {
    const stride = Math.ceil(size / 32) * 4;
    const mask = 40 + size * size * 4;
    const data = Buffer.alloc(mask + stride * size);
    data.writeUInt32LE(40, 0);
    data.writeInt32LE(size, 4);
    data.writeInt32LE(size * 2, 8);
    data.writeUInt16LE(1, 12);
    data.writeUInt16LE(32, 14);
    data.writeUInt32LE(size * size * 4, 20);
    for (let y = 0; y < size; y++) {
        for (let x = 0; x < size; x++) {
            const src = (y * size + x) * 4;
            const dest = 40 + ((size - y - 1) * size + x) * 4;
            const alpha = Math.fround(pixels[src + 3] / 255);
            const channel = i => alpha ? Math.trunc(Math.fround(Math.fround(pixels[src + i] / alpha) + 0.5)) : 0;
            data[dest] = channel(2);
            data[dest + 1] = channel(1);
            data[dest + 2] = channel(0);
            data[dest + 3] = pixels[src + 3];
            if (!pixels[src + 3]) data[mask + (size - y - 1) * stride + (x >> 3)] |= 0x80 >> (x & 7);
        }
    }
    return data;
}

try {
    const frames = sizes.map(size => {
        const rendered = new Resvg(svg, { fitTo: { mode: 'width', value: size } }).render();
        if (size === 16) return dib(rendered.pixels, size);
        const png = join(temp, `${size}.png`);
        writeFileSync(png, rendered.asPng());
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
        header.writeUInt32LE(frames[i].length, p + 8);
        header.writeUInt32LE(offset, p + 12);
        offset += frames[i].length;
    });
    writeFileSync(source, svg);
    writeFileSync(path, Buffer.concat([header, ...frames]));
    console.log(`ICO: ${old.length} -> ${offset} bytes`);
} finally {
    if (dirname(temp) !== join(root, '.tools')) throw new Error('Unexpected temporary directory');
    rmSync(temp, { recursive: true });
}
