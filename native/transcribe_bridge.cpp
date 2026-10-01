#include "transcribe.h"
#include <algorithm>
#include <cstring>
#include <exception>
#include <memory>
#include <string>
#include <stdexcept>

namespace {
thread_local std::string last_error;
struct Context {
    transcribe_model* model = nullptr;
    transcribe_session* session = nullptr;
    std::string language;
    ~Context() { transcribe_session_free(session); transcribe_model_free(model); }
};
void check(transcribe_status status) {
    if (status != TRANSCRIBE_OK) throw std::runtime_error(transcribe_status_string(status));
}
transcribe_device_t gpu(int wanted, transcribe_device_info* info) {
    int index = 0;
    for (int i = 0; i < transcribe_device_count(); ++i) {
        auto device = transcribe_device_get(i);
        transcribe_device_info_init(info);
        check(transcribe_device_get_info(device, info));
        if (info->kind && std::strcmp(info->kind, "vulkan") == 0) {
            if (index++ == wanted) return device;
        }
    }
    return nullptr;
}
}

extern "C" {
const char* vtd_native_error() noexcept { return last_error.c_str(); }
int vtd_native_gpu(int index, char* name, size_t capacity, int* discrete) noexcept {
    try {
        transcribe_device_info info;
        if (!gpu(index, &info)) return 0;
        if (!capacity) return -1;
        const char* description = info.description ? info.description : info.name;
        const size_t len = std::min(std::strlen(description), capacity - 1);
        std::memcpy(name, description, len); name[len] = 0;
        *discrete = info.device_type == TRANSCRIBE_DEVICE_TYPE_GPU;
        return 1;
    } catch (const std::exception& e) { last_error = e.what(); return -1; }
    catch (...) { last_error = "GPU discovery failed"; return -1; }
}
void* vtd_native_load(const char* path, const char* language, int threads, int device_index) noexcept {
    try {
        auto ctx = std::make_unique<Context>();
        ctx->language = language;
        transcribe_device_info info;
        transcribe_model_load_params mp;
        transcribe_model_load_params_init(&mp);
        mp.backend = TRANSCRIBE_BACKEND_VULKAN;
        mp.device = gpu(device_index, &info);
        if (!mp.device) throw std::runtime_error("Requested Vulkan GPU not available; no automatic CPU fallback");
        check(transcribe_model_load_file(path, &mp, &ctx->model));
        transcribe_session_params sp;
        transcribe_session_params_init(&sp);
        sp.n_threads = threads;
        check(transcribe_session_init(ctx->model, &sp, &ctx->session));
        return ctx.release();
    } catch (const std::exception& e) { last_error = e.what(); return nullptr; }
    catch (...) { last_error = "Model initialization failed"; return nullptr; }
}
const char* vtd_native_run(void* handle, const float* pcm, int count) noexcept {
    try {
        auto& ctx = *static_cast<Context*>(handle);
        transcribe_run_params rp;
        transcribe_run_params_init(&rp);
        rp.language = ctx.language == "auto" ? nullptr : ctx.language.c_str();
        rp.timestamps = TRANSCRIBE_TIMESTAMPS_NONE;
        check(transcribe_run(ctx.session, pcm, count, &rp));
        if (transcribe_was_truncated(ctx.session)) throw std::runtime_error("Model output was truncated; the partial transcript was not inserted");
        return transcribe_full_text(ctx.session);
    } catch (const std::exception& e) { last_error = e.what(); return nullptr; }
    catch (...) { last_error = "Transcription failed"; return nullptr; }
}
void vtd_native_free(void* handle) noexcept { delete static_cast<Context*>(handle); }
}
