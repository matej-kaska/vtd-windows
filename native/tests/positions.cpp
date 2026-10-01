#include "conformer/conformer.h"
#include "ggml-alloc.h"
#include "ggml-backend.h"
#include "ggml.h"
#include "transcribe-backend.h"

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

#define CHECK(value)                                                           \
  do {                                                                         \
    if (!(value)) {                                                            \
      std::fprintf(stderr, "FAIL line %d: %s\n", __LINE__, #value);            \
      std::abort();                                                            \
    }                                                                          \
  } while (0)

static void positions(ggml_backend_t backend) {
  for (int width : {3, 32, 1024}) {
    for (int rows : {1, 63, 64, 65, 751}) {
      auto *ctx = ggml_init({4 * ggml_tensor_overhead(), nullptr, true});
      CHECK(ctx != nullptr);
      auto *tensor = ggml_new_tensor_2d(ctx, GGML_TYPE_F32, width, rows);
      auto buffer = ggml_backend_alloc_ctx_tensors(ctx, backend);
      CHECK(buffer != nullptr);
      for (int zero : {0, 7, (rows - 1) / 2}) {
        std::vector<float> reference(static_cast<size_t>(rows) * width, 0.0f);
        std::vector<float> div(static_cast<size_t>(width / 2));
        const float ln_10000 = std::log(10000.0f);
        for (int k = 0; k < width / 2; ++k) {
          div[k] = std::exp(static_cast<float>(2 * k) *
                            (-ln_10000 / static_cast<float>(width)));
        }
        for (int i = 0; i < rows; ++i) {
          const float pos = static_cast<float>(zero - i);
          for (int k = 0; k < width / 2; ++k) {
            reference[static_cast<size_t>(i) * width + 2 * k] =
                std::sin(pos * div[k]);
            reference[static_cast<size_t>(i) * width + 2 * k + 1] =
                std::cos(pos * div[k]);
          }
        }
        transcribe::conformer::set_relative_positions(tensor, zero);
        std::vector<float> actual(reference.size());
        ggml_backend_tensor_get(tensor, actual.data(), 0,
                                actual.size() * sizeof(float));
        CHECK(std::memcmp(actual.data(), reference.data(),
                          actual.size() * sizeof(float)) == 0);
      }
      transcribe::safe_buffer_free(buffer);
      ggml_free(ctx);
    }
  }
}

static void transfers(ggml_backend_t backend) {
  constexpr size_t chunk = 1024 * 1024;
  for (size_t size :
       {size_t(4), chunk - 4, chunk, chunk + 4, 2 * chunk, 2 * chunk + 28}) {
    auto *ctx = ggml_init({4 * ggml_tensor_overhead(), nullptr, true});
    CHECK(ctx != nullptr);
    const size_t total = size + 64;
    auto *base = ggml_new_tensor_1d(ctx, GGML_TYPE_I8, total);
    auto *view = ggml_view_1d(ctx, base, size + 16, 12);
    auto buffer = ggml_backend_alloc_ctx_tensors(ctx, backend);
    CHECK(buffer != nullptr);
    std::vector<unsigned char> source(size + 1);
    for (size_t i = 0; i < source.size(); ++i) {
      source[i] = static_cast<unsigned char>((i * 31 + i / 257) & 255);
    }
    std::vector<unsigned char> expected(total, 0xa5);
    ggml_backend_tensor_set(base, expected.data(), 0, total);
    ggml_backend_tensor_set(view, source.data() + 1, 8, size);
    std::memcpy(expected.data() + 20, source.data() + 1, size);
    std::vector<unsigned char> actual(total);
    ggml_backend_tensor_get(base, actual.data(), 0, total);
    CHECK(actual == expected);
    std::vector<unsigned char> partial(size + 2, 0x7c);
    ggml_backend_tensor_get(view, partial.data() + 1, 8, size);
    CHECK(std::memcmp(partial.data() + 1, source.data() + 1, size) == 0);
    CHECK(partial.front() == 0x7c && partial.back() == 0x7c);
    transcribe::safe_buffer_free(buffer);
    ggml_free(ctx);
  }
}

int main() {
  for (const char *name : {"CPU", "Vulkan0"}) {
    auto backend = ggml_backend_init_by_name(name, nullptr);
    CHECK(backend != nullptr);
    positions(backend);
    transfers(backend);
    transcribe::safe_backend_free(backend);
  }
  std::puts(
      "90 exact position cases and 12 offset/view transfer cases passed.");
}
