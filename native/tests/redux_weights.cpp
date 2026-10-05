#include "redux_weights.h"

#include <array>
#include <cstdio>
#include <vector>

// Independent base-three encoder: position mapping, packed-byte rounding,
// group boundary and Q4 nibble ordering are checked against source trits.
static uint8_t pack(const uint8_t * codes, size_t stride, size_t count) {
    unsigned value = 0;
    for (size_t i = 0; i < count; ++i) {
        value = value * 3 + codes[i * stride];
    }
    if (count == 4) {
        value *= 3;
    }
    return static_cast<uint8_t>((value * 256 + 242) / 243);
}

int main() {
    constexpr size_t                      blocks = vtd::redux::blocks_per_chunk + 3;
    std::vector<uint8_t>                  source(blocks * vtd::redux::stored_block);
    std::vector<uint8_t>                  output(blocks * vtd::redux::runtime_block + 16, 0xa5);
    std::vector<std::array<uint8_t, 256>> expected(blocks);
    uint32_t                              rng = 0x5d2813;
    for (size_t b = 0; b < blocks; ++b) {
        uint8_t * input = source.data() + b * vtd::redux::stored_block;
        for (size_t i = 0; i < 256; ++i) {
            rng            = rng * 1664525u + 1013904223u;
            expected[b][i] = b < 3 ? uint8_t(b) : uint8_t((rng >> 16) % 3);
        }
        for (size_t g = 0; g < 2; ++g) {
            const uint8_t * c = expected[b].data() + g * 128;
            for (size_t lane = 0; lane < 16; ++lane) {
                input[g * 24 + lane] = pack(c + lane, 16, 5);
            }
            for (size_t lane = 0; lane < 8; ++lane) {
                input[g * 24 + 16 + lane] = pack(c + 80 + lane, 8, 5);
            }
            for (size_t lane = 0; lane < 2; ++lane) {
                input[48 + g * 2 + lane] = pack(c + 120 + lane, 2, 4);
            }
            input[52 + g * 2] = uint8_t(b + g * 37);
            input[53 + g * 2] = uint8_t((b % 60) + g);
        }
    }
    for (size_t at = 0; at < blocks; at += vtd::redux::blocks_per_chunk) {
        const size_t n = (blocks - at < vtd::redux::blocks_per_chunk) ? blocks - at : vtd::redux::blocks_per_chunk;
        vtd::redux::to_q4(source.data() + at * 56, output.data() + at * 144, n);
    }
    for (size_t b = 0; b < blocks; ++b) {
        for (size_t i = 0; i < 256; ++i) {
            const size_t    q    = i / 32;
            const uint8_t * out  = output.data() + b * 144 + q * 18;
            const uint8_t * in   = source.data() + b * 56 + 52 + (i / 128) * 2;
            const unsigned  code = ((out[2 + i % 16] >> (i % 32 >= 16 ? 4 : 0)) & 15) - 7;
            if (code != expected[b][i] || out[0] != in[0] || out[1] != in[1]) {
                return 1;
            }
        }
    }
    for (size_t i = blocks * 144; i < output.size(); ++i) {
        if (output[i] != 0xa5) {
            return 2;
        }
    }
    std::printf("Redux: %zu weight codes and scale bits preserved across chunk boundaries\n", blocks * 256);
}
