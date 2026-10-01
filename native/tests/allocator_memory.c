#include "ggml-alloc.c"
#include "ggml-cpu.h"

#define CHECK(value)                                                           \
  do {                                                                         \
    if (!(value)) {                                                            \
      fprintf(stderr, "FAIL %d: %s\n", __LINE__, #value);                      \
      abort();                                                                 \
    }                                                                          \
  } while (0)

static void exercise(ggml_backend_t backend) {
  ggml_backend_buffer_type_t buft =
      ggml_backend_get_default_buffer_type(backend);
  ggml_gallocr_t alloc = ggml_gallocr_new(buft);
  struct ggml_context *inputs = ggml_init(
      (struct ggml_init_params){4 * ggml_tensor_overhead(), NULL, true});
  CHECK(inputs);
  struct ggml_tensor *step = ggml_new_tensor_1d(inputs, GGML_TYPE_F32, 32);
  ggml_backend_buffer_t buffer =
      ggml_backend_alloc_ctx_tensors(inputs, backend);
  CHECK(buffer);
  float increment[32];
  for (int i = 0; i < 32; i++)
    increment[i] = (float)(i % 4 + 1) * 0.25f;
  ggml_backend_tensor_set(step, increment, 0, sizeof(increment));
  const int counts[] = {0, 4, 513, 7, 5048, 9};
  const uint32_t masks[] = {0,      1u << 9, 1u << 2, (1u << 9) | (1u << 2),
                            0x3fcu, 0};
  for (size_t c = 0; c < sizeof(counts) / sizeof(counts[0]); c++) {
    const int count = counts[c];
    for (size_t m = 0; m < sizeof(masks) / sizeof(masks[0]); m++) {
      struct ggml_context *ctx = ggml_init((struct ggml_init_params){
          (size_t)(count + 8) * ggml_tensor_overhead() +
              ggml_graph_overhead_custom(8192, false),
          NULL, true});
      CHECK(ctx);
      struct ggml_tensor *input = ggml_new_tensor_1d(ctx, GGML_TYPE_F32, 32);
      ggml_set_input(input);
      struct ggml_tensor *view = ggml_view_1d(ctx, step, 32, 0);
      struct ggml_tensor *value = input;
      for (int i = 0; i < count; i++) {
        value = ggml_add(ctx, value, i % 2 ? view : step);
        for (int j = 2; j < GGML_MAX_SRC; j++) {
          if (masks[m] & (1u << j))
            value->src[j] = step;
        }
      }
      ggml_set_output(value);
      struct ggml_cgraph *graph = ggml_new_graph_custom(ctx, 8192, false);
      ggml_build_forward_expand(graph, value);
      CHECK(ggml_gallocr_alloc_graph(alloc, graph));
      if (count == 5048 && m == 0) {
        const size_t old_size = (size_t)graph->n_nodes * (GGML_MAX_SRC + 1) *
                                sizeof(struct tensor_alloc);
        const size_t new_size =
            (size_t)graph->n_nodes * sizeof(struct node_alloc) +
            alloc->src_capacity * sizeof(struct tensor_alloc);
        printf("%s nodes=%d previous_metadata=%zu packed_metadata=%zu\n",
               ggml_backend_name(backend), graph->n_nodes, old_size, new_size);
        CHECK(new_size * 2 < old_size);
      }
      for (int repeat = 0; repeat < 2; repeat++) {
        float source[32], result[32];
        for (int i = 0; i < 32; i++)
          source[i] = (float)(i + repeat);
        ggml_backend_tensor_set(input, source, 0, sizeof(source));
        CHECK(ggml_backend_graph_compute(backend, graph) ==
              GGML_STATUS_SUCCESS);
        ggml_backend_tensor_get(value, result, 0, sizeof(result));
        for (int i = 0; i < 32; i++)
          CHECK(result[i] == source[i] + count * increment[i]);
      }
      if (count) {
        value->src[9] = value->src[9] ? NULL : step;
        CHECK(ggml_gallocr_needs_realloc(alloc, graph));
        CHECK(ggml_gallocr_alloc_graph(alloc, graph));
        CHECK(!ggml_gallocr_needs_realloc(alloc, graph));
        float source[32] = {0}, result[32];
        ggml_backend_tensor_set(input, source, 0, sizeof(source));
        CHECK(ggml_backend_graph_compute(backend, graph) ==
              GGML_STATUS_SUCCESS);
        ggml_backend_tensor_get(value, result, 0, sizeof(result));
        for (int i = 0; i < 32; i++)
          CHECK(result[i] == count * increment[i]);
      }
      ggml_free(ctx);
    }
  }
  ggml_gallocr_free(alloc);
  ggml_backend_buffer_free(buffer);
  ggml_free(inputs);
}

int main(void) {
  ggml_backend_load_all();
  ggml_backend_t cpu = ggml_backend_cpu_init();
  CHECK(cpu);
  ggml_backend_cpu_set_n_threads(cpu, 2);
  exercise(cpu);
  ggml_backend_dev_t device = ggml_backend_dev_by_name("Vulkan0");
  CHECK(device);
  ggml_backend_t gpu = ggml_backend_dev_init(device, NULL);
  CHECK(gpu);
  exercise(gpu);
  ggml_backend_free(gpu);
  ggml_backend_free(cpu);
  puts("Packed allocator: 72 graphs, sparse/last/all source slots, views, "
       "external tensors and changed source masks passed.");
  return 0;
}
