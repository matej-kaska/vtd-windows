import pathlib
import struct
import subprocess
import sys
import zlib

source, destination, seven_zip = sys.argv[1:]
data = pathlib.Path(source).read_bytes()
root = pathlib.Path(destination)
root.mkdir(parents=True, exist_ok=True)
position = 0
count = 0
while True:
    start = data.find(b"7z\xbc\xaf\x27\x1c", position)
    if start < 0:
        break
    position = start + 6
    if start + 32 > len(data) or zlib.crc32(data[start+12:start+32]) != struct.unpack_from("<I", data, start+8)[0]:
        continue
    offset, size = struct.unpack_from("<QQ", data, start+12)
    end = start + 32 + offset + size
    if end > len(data):
        continue
    archive = root / "sdk-part.7z"
    archive.write_bytes(data[start:end])
    subprocess.run([seven_zip, "x", str(archive), f"-o{root}", "-y", "-bso0", "-bsp0"], check=True)
    archive.unlink()
    count += 1
if count == 0 or not (root / "Bin/glslc.exe").exists():
    raise RuntimeError("Vulkan SDK extraction failed")
