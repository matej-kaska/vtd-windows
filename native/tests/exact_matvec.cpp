#include "exact_matvec.h"
#include "ggml-alloc.h"
#include "ggml-backend.h"
#include "ggml-cpu.h"
#include "gguf.h"
#include "transcribe-backend.h"

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <vector>

#define CHECK(condition)                                                       \
  do {                                                                         \
    if (!(condition)) {                                                        \
      std::fprintf(stderr, "FAIL line %d: %s\n", __LINE__, #condition);        \
      std::abort();                                                            \
    }                                                                          \
  } while (0)

// Oracle: the existing CPU fp32 MUL_MAT over a full reference expansion.
// This also catches future changes to GGML's accumulation/reduction order.
static void compare(ggml_backend_t cpu, int columns, int rows,
                    const std::vector<uint8_t> &raw) {
  std::vector<float> expanded(static_cast<size_t>(columns) * rows);
  ggml_get_type_traits(GGML_TYPE_Q8_0)
      ->to_float(raw.data(), expanded.data(), expanded.size());
  ggml_init_params params{
      12 * ggml_tensor_overhead() + 2 * ggml_graph_overhead(), nullptr, true};
  auto *ctx = ggml_init(params);
  CHECK(ctx != nullptr);
  auto *weights = ggml_new_tensor_2d(ctx, GGML_TYPE_Q8_0, columns, rows);
  auto *fp32 = ggml_new_tensor_2d(ctx, GGML_TYPE_F32, columns, rows);
  auto *input = ggml_new_tensor_1d(ctx, GGML_TYPE_F32, columns);
  CHECK(vtd_exact_q8_supported(weights));
  CHECK(!vtd_exact_q8_supported(fp32));
  auto *ref = ggml_mul_mat(ctx, fp32, input);
  auto *result = vtd_exact_q8_mul_mat(ctx, weights, input);
  auto *buffer = ggml_backend_alloc_ctx_tensors(ctx, cpu);
  CHECK(buffer != nullptr);
  ggml_backend_tensor_set(weights, raw.data(), 0, raw.size());
  ggml_backend_tensor_set(fp32, expanded.data(), 0,
                          expanded.size() * sizeof(float));
  auto *reference = ggml_new_graph(ctx);
  auto *proposed = ggml_new_graph(ctx);
  ggml_build_forward_expand(reference, ref);
  ggml_build_forward_expand(proposed, result);
  uint32_t random = 123456789;
  std::vector<float> input_data(columns), expected(rows), actual(rows);
  for (int threads : {1, 4}) {
    ggml_backend_cpu_set_n_threads(cpu, threads);
    for (int k = 0; k < 32; ++k) {
      for (auto &value : input_data) {
        random = random * 1664525 + 1013904223;
        value = k == 0 ? 0.0f
                       : static_cast<float>(static_cast<int32_t>(random)) /
                             2147483648.0f;
      }
      ggml_backend_tensor_set(input, input_data.data(), 0,
                              input_data.size() * sizeof(float));
      CHECK(ggml_backend_graph_compute(cpu, reference) == GGML_STATUS_SUCCESS);
      CHECK(ggml_backend_graph_compute(cpu, proposed) == GGML_STATUS_SUCCESS);
      ggml_backend_tensor_get(ref, expected.data(), 0,
                              expected.size() * sizeof(float));
      ggml_backend_tensor_get(result, actual.data(), 0,
                              actual.size() * sizeof(float));
      CHECK(std::memcmp(expected.data(), actual.data(),
                        actual.size() * sizeof(float)) == 0);
    }
  }
  std::printf("exact matvec: %d x %d, 64 input/thread combinations passed\n",
              rows, columns);
  transcribe::safe_buffer_free(buffer);
  ggml_free(ctx);
}

int main() {
  const char *path = std::getenv("TRANSCRIBE_PARAKEET_GGUF");
  if (path == nullptr || *path == '\0') {
    std::puts("Set TRANSCRIBE_PARAKEET_GGUF to run the exact matvec oracle");
    return 77;
  }
  auto *cpu = ggml_backend_cpu_init();
  CHECK(cpu != nullptr);
  // Include widths with one block and an odd block count, and rows smaller
  // than the threadpool. Quantize once, then compare the same stored weights.
  for (int columns : {32, 64, 96, 640, 1024}) {
    constexpr int rows = 17;
    std::vector<float> original(columns * rows);
    uint32_t random = 123;
    for (auto &value : original) {
      random = random * 1664525 + 1013904223;
      value = static_cast<float>(static_cast<int32_t>(random)) / 2147483648.0f;
    }
    std::vector<uint8_t> raw(ggml_row_size(GGML_TYPE_Q8_0, columns) * rows);
    CHECK(ggml_quantize_chunk(GGML_TYPE_Q8_0, original.data(), raw.data(), 0,
                              rows, columns, nullptr) == raw.size());
    compare(cpu, columns, rows, raw);
    raw.resize(ggml_row_size(GGML_TYPE_Q8_0, columns));
    compare(cpu, columns, 1, raw);
  }

  ggml_context *metadata = nullptr;
  gguf_init_params params{true, &metadata};
  auto *gguf = gguf_init_from_file(path, params);
  CHECK(gguf != nullptr);
  for (const char *name :
       {"joint.out.weight", "joint.pred.weight", "pred.lstm.0.Wx",
        "pred.lstm.0.Wh", "pred.lstm.1.Wx", "pred.lstm.1.Wh"}) {
    const auto *weight = ggml_get_tensor(metadata, name);
    CHECK(weight != nullptr && weight->type == GGML_TYPE_Q8_0);
    std::vector<uint8_t> raw(ggml_nbytes(weight));
    std::ifstream stream(path, std::ios::binary);
    stream.seekg(gguf_get_data_offset(gguf) +
                 gguf_get_tensor_offset(gguf, gguf_find_tensor(gguf, name)));
    stream.read(reinterpret_cast<char *>(raw.data()), raw.size());
    CHECK(stream.good());
    std::printf("%s: ", name);
    compare(cpu, static_cast<int>(weight->ne[0]),
            static_cast<int>(weight->ne[1]), raw);
  }
  ggml_free(metadata);
  gguf_free(gguf);
  transcribe::safe_backend_free(cpu);
  std::puts("exact compact matvec checks passed");
}
