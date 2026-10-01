#pragma once
#include "ggml.h"

bool vtd_exact_q8_supported(const ggml_tensor *weights) noexcept;
ggml_tensor *vtd_exact_q8_mul_mat(ggml_context *ctx, ggml_tensor *weights,
                                  ggml_tensor *input);
