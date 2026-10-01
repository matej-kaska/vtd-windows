#include "ggml-alloc.h"
#include "ggml-backend.h"
#include "ggml-cpu.h"
#include "ggml.h"
#include "transcribe-backend.h"

#define NOMINMAX
#include <windows.h>

#include <psapi.h>

#include <array>
#include <cstdio>
#include <cstdlib>

#define CHECK(condition)                                                       \
  do {                                                                         \
    if (!(condition)) {                                                        \
      std::fprintf(stderr, "FAIL line %d: %s\n", __LINE__, #condition);        \
      std::abort();                                                            \
    }                                                                          \
  } while (0)

static SIZE_T private_commit() {
  PROCESS_MEMORY_COUNTERS_EX memory{};
  CHECK(GetProcessMemoryInfo(
      GetCurrentProcess(), reinterpret_cast<PROCESS_MEMORY_COUNTERS *>(&memory),
      sizeof(memory)));
  return memory.PrivateUsage;
}

// Exercise arena growth, graph growth/shrinkage, changing backend assignments
// and pipelined input copies. Every operation is exact for these dyadic inputs.
static void exercise(ggml_backend_t gpu, ggml_backend_t cpu, bool parallel) {
  ggml_backend_t backends[] = {gpu, cpu};
  const SIZE_T before = private_commit();
  auto *scheduler =
      ggml_backend_sched_new(gpu ? backends : backends + 1, nullptr,
                             gpu ? 2 : 1, 8192, parallel, true);
  CHECK(scheduler != nullptr);
  const SIZE_T after = private_commit();
  const SIZE_T extra = after > before ? after - before : 0;
  std::printf("scheduler %s parallel=%d initial commit: %llu bytes\n",
              gpu ? "Vulkan+CPU" : "CPU", parallel,
              static_cast<unsigned long long>(extra));
  // The former worst-case reservation was over 200 MiB for this capacity.
  CHECK(extra < 16 * 1024 * 1024);

  ggml_init_params inputs_params{8 * ggml_tensor_overhead(), nullptr, true};
  auto *inputs_ctx = ggml_init(inputs_params);
  CHECK(inputs_ctx != nullptr);
  auto *input = ggml_new_tensor_1d(inputs_ctx, GGML_TYPE_F32, 32);
  auto *increment = ggml_new_tensor_1d(inputs_ctx, GGML_TYPE_F32, 32);
  ggml_set_input(input);
  ggml_set_input(increment);
  auto *input_buffer = ggml_backend_alloc_ctx_tensors(inputs_ctx, cpu);
  CHECK(input_buffer != nullptr);

  for (int count : {2, 385, 7, 513, 1, 300}) {
    ggml_backend_sched_reset(scheduler);
    ggml_init_params graph_params{(static_cast<size_t>(count) + 8) *
                                          ggml_tensor_overhead() +
                                      ggml_graph_overhead_custom(2048, false),
                                  nullptr, true};
    auto *context = ggml_init(graph_params);
    CHECK(context != nullptr);
    auto *value = input;
    for (int i = 0; i < count; ++i) {
      value = ggml_add(context, value, increment);
      ggml_backend_sched_set_tensor_backend(
          scheduler, value, gpu && (i % 2 == count % 2) ? gpu : cpu);
    }
    ggml_set_output(value);
    auto *graph = ggml_new_graph_custom(context, 2048, false);
    ggml_build_forward_expand(graph, value);
    CHECK(ggml_backend_sched_alloc_graph(scheduler, graph));
    for (int repeat = 0; repeat < 3; ++repeat) {
      std::array<float, 32> source{}, step{}, result{};
      for (size_t i = 0; i < source.size(); ++i) {
        source[i] = static_cast<float>(i + repeat);
        step[i] = static_cast<float>((i % 4) + 1) * 0.25f;
      }
      ggml_backend_tensor_set(input, source.data(), 0, sizeof(source));
      ggml_backend_tensor_set(increment, step.data(), 0, sizeof(step));
      CHECK(ggml_backend_sched_graph_compute(scheduler, graph) ==
            GGML_STATUS_SUCCESS);
      ggml_backend_tensor_get(value, result.data(), 0, sizeof(result));
      for (size_t i = 0; i < source.size(); ++i) {
        CHECK(result[i] == source[i] + static_cast<float>(count) * step[i]);
      }
    }
    ggml_backend_sched_synchronize(scheduler);
    ggml_backend_sched_reset(scheduler);
    ggml_free(context);
  }
  transcribe::safe_sched_free(scheduler);
  transcribe::safe_buffer_free(input_buffer);
  ggml_free(inputs_ctx);
}

int main() {
  ggml_backend_load_all();
  auto *cpu = ggml_backend_cpu_init();
  CHECK(cpu != nullptr);
  ggml_backend_cpu_set_n_threads(cpu, 2);
  exercise(nullptr, cpu, false);
  exercise(nullptr, cpu, true);
  auto *device = ggml_backend_dev_by_name("Vulkan0");
  CHECK(device != nullptr);
  auto *gpu = ggml_backend_dev_init(device, nullptr);
  CHECK(gpu != nullptr);
  exercise(gpu, cpu, false);
  exercise(gpu, cpu, true);
  transcribe::safe_backend_free(gpu);
  transcribe::safe_backend_free(cpu);
  std::puts("scheduler memory and transfer checks passed");
}
