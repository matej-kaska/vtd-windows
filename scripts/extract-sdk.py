import mmap
import pathlib
import struct
import subprocess
import sys
import zlib

source, destination, seven_zip = sys.argv[1:]
root = pathlib.Path(destination)
root.mkdir(parents=True, exist_ok=True)
count = 0
with open(source, "rb") as file, mmap.mmap(file.fileno(), 0, access=mmap.ACCESS_READ) as data:
    position = 0
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
        try:
            with archive.open("wb") as output:
                for block in range(start, end, 1024 * 1024):
                    output.write(data[block:min(block + 1024 * 1024, end)])
            subprocess.run([seven_zip, "x", str(archive), f"-o{root}", "-y", "-bso0", "-bsp0", "-mmt=2",
                            "Bin/glslc.exe", "Lib/vulkan-1.lib", "Include/vulkan/*", "Include/vk_video/*"], check=True)
        finally:
            archive.unlink(missing_ok=True)
        count += 1
if count == 0 or not all((root / name).is_file() for name in ("Bin/glslc.exe", "Lib/vulkan-1.lib", "Include/vulkan/vulkan.hpp")):
    raise RuntimeError("Vulkan SDK extraction failed")
