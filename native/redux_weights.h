#pragma once

#include <cstddef>
#include <cstdint>
#include <cstring>

// TQ1_G128 layout and exact Q4_0 repacking adapted from NairoDorian/transcribe.cpp
// ba949120d60f29daaaa13eec65b9c28c2c2112a6 (MIT). This is a storage conversion;
// all {-1, 0, +1} codes and FP16 scale bits are preserved without requantization.
namespace vtd::redux {
constexpr size_t stored_block       = 56;
constexpr size_t runtime_block      = 144;
constexpr size_t elements_per_block = 256;
constexpr size_t staging_limit      = 1024 * 1024;
constexpr size_t blocks_per_chunk   = staging_limit / (stored_block + runtime_block);

inline void to_q4(const uint8_t * source, uint8_t * target, size_t blocks) {
    constexpr uint8_t powers[] = { 1, 3, 9, 27, 81 };
    for (size_t block = 0; block < blocks; ++block) {
        const uint8_t * input  = source + block * stored_block;
        uint8_t *       output = target + block * runtime_block;
        for (size_t group = 0; group < 2; ++group) {
            uint8_t codes[128];
            for (size_t digit = 0; digit < 5; ++digit) {
                for (size_t lane = 0; lane < 16; ++lane) {
                    const uint8_t q          = static_cast<uint8_t>(input[group * 24 + lane] * powers[digit]);
                    codes[lane + 16 * digit] = static_cast<uint8_t>((uint16_t(q) * 3) >> 8);
                }
                for (size_t lane = 0; lane < 8; ++lane) {
                    const uint8_t q              = static_cast<uint8_t>(input[group * 24 + 16 + lane] * powers[digit]);
                    codes[80 + lane + 8 * digit] = static_cast<uint8_t>((uint16_t(q) * 3) >> 8);
                }
            }
            for (size_t digit = 0; digit < 4; ++digit) {
                for (size_t lane = 0; lane < 2; ++lane) {
                    const uint8_t q               = static_cast<uint8_t>(input[48 + group * 2 + lane] * powers[digit]);
                    codes[120 + lane + 2 * digit] = static_cast<uint8_t>((uint16_t(q) * 3) >> 8);
                }
            }
            for (size_t part = 0; part < 4; ++part) {
                uint8_t * q4 = output + (group * 4 + part) * 18;
                std::memcpy(q4, input + 52 + group * 2, 2);
                for (size_t lane = 0; lane < 16; ++lane) {
                    q4[2 + lane] =
                        static_cast<uint8_t>((codes[part * 32 + lane] + 7) | ((codes[part * 32 + lane + 16] + 7) << 4));
                }
            }
        }
    }
}
}  // namespace vtd::redux
