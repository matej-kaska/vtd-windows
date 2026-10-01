#include "arch/canary/canary.h"
#include "arch/canary/decoder.h"
#include "arch/canary/encoder.h"
#include "conformer/conformer.h"
#include "ggml-alloc.h"
#include "ggml-backend.h"
#include "ggml.h"
#include "transcribe.h"

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

using namespace transcribe;
using namespace transcribe::canary;

struct Result {
  std::vector<float> encoder;
  std::vector<unsigned char> cross_k, cross_v;
  size_t encoder_cpu, cross_cpu;
};

static Result pipeline(CanaryModel &model, int frames, bool direct) {
  auto *ctx = ggml_init({8 * 1024 * 1024, nullptr, true});
  CHECK(ctx != nullptr);
  auto eb = build_encoder_graph(ctx, model.weights, model.hparams, frames,
                                GGML_TYPE_F16, true, model.backend.c_str());
  CHECK(eb.graph && eb.out);
  auto &backends = model.plan.scheduler_list;
  auto sched = ggml_backend_sched_new(backends.data(), nullptr,
                                      static_cast<int>(backends.size()), 16384,
                                      false, true);
  CHECK(sched != nullptr);
  if (direct) {
    ggml_backend_sched_set_tensor_backend(sched, eb.mel_in, model.plan.primary);
    ggml_backend_sched_set_tensor_backend(sched, eb.pos_emb_in,
                                          model.plan.primary);
  }
  CHECK(ggml_backend_sched_alloc_graph(sched, eb.graph));
  Result result;
  result.encoder_cpu =
      ggml_backend_sched_get_buffer_size(sched, backends.back());
  std::vector<float> mel(static_cast<size_t>(ggml_nelements(eb.mel_in)));
  for (size_t i = 0; i < mel.size(); ++i) {
    mel[i] =
        static_cast<float>(static_cast<int>((i * 13) % 97) - 48) * 0.03125f;
  }
  ggml_backend_tensor_set(eb.mel_in, mel.data(), 0, mel.size() * sizeof(float));
  conformer::set_relative_positions(
      eb.pos_emb_in, static_cast<int>((eb.pos_emb_in->ne[1] - 1) / 2));
  CHECK(ggml_backend_sched_graph_compute(sched, eb.graph) ==
        GGML_STATUS_SUCCESS);
  const int encoded_frames = static_cast<int>(eb.out->ne[1]);
  result.encoder.resize(static_cast<size_t>(ggml_nelements(eb.out)));
  ggml_backend_tensor_get(eb.out, result.encoder.data(), 0,
                          result.encoder.size() * sizeof(float));
  std::printf(
      "frames=%d direct=%d encoder_nodes=%d metadata=%zu cpu_buffer=%zu\n",
      frames, direct, ggml_graph_n_nodes(eb.graph), ggml_used_mem(ctx),
      result.encoder_cpu);

  auto *saved_ctx = ggml_init({ggml_tensor_overhead(), nullptr, true});
  CHECK(saved_ctx != nullptr);
  auto *saved = ggml_dup_tensor(saved_ctx, eb.out);
  auto saved_buffer =
      ggml_backend_alloc_ctx_tensors(saved_ctx, model.plan.primary);
  CHECK(saved_buffer != nullptr);
  ggml_backend_tensor_copy(eb.out, saved);
  ggml_backend_sched_reset(sched);
  ggml_free(ctx);

  CanaryKvCache kv;
  CHECK(kv_cache_init(kv, model.plan.primary, 1024, encoded_frames,
                      model.hparams.dec_d_model, model.hparams.dec_n_layers,
                      GGML_TYPE_F16));
  ctx = ggml_init({4 * 1024 * 1024, nullptr, true});
  CHECK(ctx != nullptr);
  auto cross = build_cross_kv_graph(ctx, model.weights, model.hparams, kv,
                                    encoded_frames, direct ? saved : nullptr);
  CHECK(cross.graph != nullptr);
  CHECK(ggml_backend_sched_alloc_graph(sched, cross.graph));
  result.cross_cpu = ggml_backend_sched_get_buffer_size(sched, backends.back());
  if (!direct) {
    ggml_backend_tensor_set(cross.encoder_out_in, result.encoder.data(), 0,
                            result.encoder.size() * sizeof(float));
  }
  CHECK(ggml_backend_sched_graph_compute(sched, cross.graph) ==
        GGML_STATUS_SUCCESS);
  result.cross_k.resize(ggml_nbytes(kv.cross_k));
  result.cross_v.resize(ggml_nbytes(kv.cross_v));
  ggml_backend_tensor_get(kv.cross_k, result.cross_k.data(), 0,
                          result.cross_k.size());
  ggml_backend_tensor_get(kv.cross_v, result.cross_v.data(), 0,
                          result.cross_v.size());
  std::printf(
      "frames=%d direct=%d cross_nodes=%d metadata=%zu cpu_buffer=%zu\n",
      frames, direct, ggml_graph_n_nodes(cross.graph), ggml_used_mem(ctx),
      result.cross_cpu);
  safe_sched_free(sched);
  ggml_free(ctx);
  safe_buffer_free(saved_buffer);
  ggml_free(saved_ctx);
  kv.free();
  return result;
}

int main() {
  const char *path = std::getenv("TRANSCRIBE_CANARY_1B_V2_GGUF");
  if (!path || !*path) {
    return 77;
  }
  transcribe_model_load_params params;
  transcribe_model_load_params_init(&params);
  params.backend = TRANSCRIBE_BACKEND_VULKAN;
  transcribe_model *raw = nullptr;
  CHECK(transcribe_model_load_file(path, &params, &raw) == TRANSCRIBE_OK);
  auto &model = *static_cast<CanaryModel *>(raw);
  for (int frames : {17, 1411, 3001}) {
    auto reference = pipeline(model, frames, false);
    auto candidate = pipeline(model, frames, true);
    CHECK(reference.encoder.size() == candidate.encoder.size());
    CHECK(std::memcmp(reference.encoder.data(), candidate.encoder.data(),
                      reference.encoder.size() * sizeof(float)) == 0);
    CHECK(reference.cross_k == candidate.cross_k &&
          reference.cross_v == candidate.cross_v);
    CHECK(candidate.encoder_cpu < reference.encoder_cpu);
    CHECK(candidate.cross_cpu < reference.cross_cpu);
  }
  transcribe_model_free(raw);
  std::puts("Canary encoder and cross-KV match bit for bit at all 3 lengths; "
            "CPU buffers shrink.");
}
