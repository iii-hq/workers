// The slice of llama.cpp the judge providers call, as a C API of scalars and
// pointers (src/lib.rs declares it by hand): llama.h passes structs by value,
// and llama_batch_ext_set_decision_order is C++ (src/llama-ext.h).
#include "ggml-backend.h"
#include "llama.h"
#include "llama-ext.h"

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <exception>
#include <string>
#include <strings.h>
#include <vector>

// llama.cpp logs every tensor while loading (~1300 lines): keep warnings and
// errors. A continuation line follows its message's fate. Also dropped: the
// context's "compute buffer size ... does not match expectation" (one line per
// backend), which llama.cpp logs on freeing a context whose pass reallocated
// its compute buffers, as 16k clef prompts do (judge-clef/README.md: the 14.1
// GiB peak).
static void log_warnings(ggml_log_level level, const char * text, void *) {
    thread_local bool keep = false;
    if (level != GGML_LOG_LEVEL_CONT) {
        keep = level >= GGML_LOG_LEVEL_WARN &&
               !(strncmp(text, "~llama_context:", 15) == 0 && strstr(text, "does not match expectation"));
    }
    if (keep) {
        fputs(text, stderr);
    }
}

// Load the backend modules (GGML_BACKEND_DL builds) from `dir`, else from
// `fallback` when `dir` had none; both may be null. A vk_max_nodes_per_submit
// other than 0 sets GGML_VK_MAX_NODES_PER_SUBMIT first, unless the operator
// did; 0 leaves the environment alone.
extern "C" void ln_backend_init(const char * dir, const char * fallback, uint32_t vk_max_nodes_per_submit) {
    if (vk_max_nodes_per_submit) {
        setenv("GGML_VK_MAX_NODES_PER_SUBMIT", std::to_string(vk_max_nodes_per_submit).c_str(), 0);
    }
    llama_log_set(log_warnings, nullptr);
    if (dir) {
        ggml_backend_load_all_from_path(dir);
    }
    if (fallback && ggml_backend_dev_count() == 0) {
        ggml_backend_load_all_from_path(fallback);
    }
    llama_backend_init();
}

// "<description> (<backend>)" of the first device that is not the CPU.
extern "C" bool ln_gpu_device(char * out, size_t len) {
    for (size_t i = 0; i < ggml_backend_dev_count(); i++) {
        ggml_backend_dev_t dev = ggml_backend_dev_get(i);
        const char * backend = ggml_backend_reg_name(ggml_backend_dev_backend_reg(dev));
        if (strcasecmp(backend, "CPU") != 0) {
            snprintf(out, len, "%s (%s)", ggml_backend_dev_description(dev), backend);
            return true;
        }
    }
    return false;
}

// gpu_layers: -1 every layer, n that many. use_gpu_devices false loads with no
// GPU device at all; true leaves them to llama.cpp, which with gpu_layers 0
// still keeps CPU weights in a GPU's pinned host buffer (unrepacked) and sends
// it large batch matmuls.
extern "C" llama_model * ln_model_load(const char * path, int32_t gpu_layers, bool use_gpu_devices) {
    llama_model_params params = llama_model_default_params();
    ggml_backend_dev_t no_devices[] = { nullptr };
    params.n_gpu_layers = gpu_layers;
    if (!use_gpu_devices) {
        params.devices = no_devices;
    }
    return llama_model_load_from_file(path, params);
}

// A context over `model` (llama_context_params is passed by value): the
// fields below, the rest at their defaults. pooling_type is llama.h's enum
// value. Null when llama.cpp refuses the parameters.
extern "C" llama_context * ln_context_new(llama_model * model, uint32_t n_ctx, uint32_t n_batch, uint32_t n_ubatch,
                                          uint32_t n_seq_max, int32_t n_threads, bool embeddings,
                                          int32_t pooling_type, bool kv_unified) {
    // llama.cpp asserts (aborts) when the sequences outnumber the output rows
    // it reserves: n_batch, clamped to n_ctx for a causal model.
    const uint32_t n_ctx_eff = n_ctx ? n_ctx : (uint32_t) llama_model_n_ctx_train(model);
    if (n_threads < 1 || std::max(1u, n_seq_max) > std::min(n_batch, n_ctx_eff)) {
        return nullptr;
    }
    llama_context_params params = llama_context_default_params();
    params.n_ctx           = n_ctx;
    params.n_batch         = n_batch;
    params.n_ubatch        = n_ubatch;
    params.n_seq_max       = n_seq_max;
    params.n_threads       = params.n_threads_batch = n_threads;
    params.embeddings      = embeddings;
    params.pooling_type    = (enum llama_pooling_type) pooling_type;
    params.kv_unified      = kv_unified;
    return llama_init_from_model(model, params);
}

// One llama_encode (encode) or llama_decode of n_tokens tokens: token i at
// position pos[i] of sequence seq[i], its logits or embeddings kept when
// output[i] is not 0. Returns -1001 for what llama.cpp would abort the process
// on: no tokens, more than n_batch (n_ubatch when the pass is one micro-batch:
// an encode, or a context without memory or causal attention), an encode on a
// context with memory (llama.cpp's encoder graph gets no memory context), a
// sequence outside [0, n_seq_max) or a negative position. -1006 when the pass
// throws (e.g. a Vulkan allocation), else llama.cpp's own code: 0 done, 1 no
// memory slot, 2 aborted, -1 a batch it refuses (a token outside the
// vocabulary, positions that do not continue their sequence), -2/-3 compute
// failures.
extern "C" int32_t ln_process(llama_context * ctx, bool encode, const int32_t * tokens, const int32_t * pos,
                              const int32_t * seq, const int8_t * output, int32_t n_tokens) {
    const bool one_ubatch = encode || !llama_get_memory(ctx) || !llama_get_causal_attn(ctx);
    if ((encode && llama_get_memory(ctx)) || n_tokens < 1 ||
        (uint32_t) n_tokens > (one_ubatch ? llama_n_ubatch(ctx) : llama_n_batch(ctx))) {
        return -1001;
    }
    const llama_seq_id n_seq_max = (llama_seq_id) llama_n_seq_max(ctx);
    try {
        std::vector<int32_t>        n_seq_id(n_tokens, 1);
        std::vector<llama_seq_id *> seq_id(n_tokens);
        for (int32_t i = 0; i < n_tokens; i++) {
            if (seq[i] < 0 || seq[i] >= n_seq_max || pos[i] < 0) {
                return -1001;
            }
            seq_id[i] = const_cast<llama_seq_id *>(&seq[i]);
        }
        const llama_batch batch = {
            n_tokens, const_cast<llama_token *>(tokens), nullptr, const_cast<llama_pos *>(pos),
            n_seq_id.data(), seq_id.data(), const_cast<int8_t *>(output),
        };
        return encode ? llama_encode(ctx, batch) : llama_decode(ctx, batch);
    } catch (const std::exception & e) {
        fprintf(stderr, "ln_process: %s\n", e.what());
        return -1006;
    } catch (...) {
        return -1006;
    }
}

// One forward of a decision model (llama.cpp's `clef` arch: the head in the
// graph) over the whole prompt in a context sized to it, freed before
// returning: n_ctx = n_batch = n_ubatch = n_tokens rounded up to 256,
// the padding llama.cpp gives n_ctx anyway (it aborts the process on an
// encode longer than n_ubatch). orders[i] is the token's
// llama_decision_order (0 none, 1-3 question noul/choice/score, 4 option);
// scores[k] gets option k's score, options in prompt order.
// Returns 0, -1001 on bad input, -1002 when the context cannot be created,
// -1003 when the batch refuses a token, -1004 with no score, -1005 when the
// GGUF sets a pooling type, -1006 when the pass throws (e.g. a Vulkan
// allocation), else llama_process's own code.
extern "C" int32_t ln_decide(const llama_model * model, int32_t n_threads, const int32_t * ids,
                             const uint8_t * orders, int32_t n_tokens, float * scores, int32_t n_scores) {
    int32_t n_options = 0;
    for (int32_t i = 0; i < n_tokens; i++) {
        if (orders[i] > LLAMA_DECISION_ORDER_OPTION) {
            return -1001;
        }
        n_options += orders[i] == LLAMA_DECISION_ORDER_OPTION && (i == 0 || orders[i - 1] != orders[i]);
    }
    if (n_tokens <= 0 || n_scores <= 0 || n_options != n_scores) {
        return -1001;
    }
    llama_context_params params = llama_context_default_params();
    params.n_ctx = params.n_batch = params.n_ubatch = (uint32_t) ((n_tokens + 255) / 256 * 256);
    params.n_seq_max       = 1;
    params.n_threads       = params.n_threads_batch = n_threads;
    // Pooling stays the GGUF's, none for clef (asking for NONE logs a warning).
    params.embeddings      = true;
    // Without flash attention the n_ctx x n_ubatch scores outgrow the GPU at 8k tokens.
    params.flash_attn_type = LLAMA_FLASH_ATTN_TYPE_ENABLED;
    llama_context * ctx = llama_init_from_model(const_cast<llama_model *>(model), params);
    if (!ctx) {
        return -1002;
    }
    // A GGUF with a pooling type would pool the scores into embd_seq and leave every row 0.
    if (llama_pooling_type(ctx) != LLAMA_POOLING_TYPE_NONE) {
        llama_free(ctx);
        return -1005;
    }
    llama_batch_ext * batch = llama_batch_ext_init(ctx);
    int32_t status = 0;
    for (int32_t i = 0; i < n_tokens && status == 0; i++) {
        const llama_pos pos = i;
        if (llama_batch_ext_add_token(batch, 0, ids[i]) != i || !llama_batch_ext_set_pos(batch, i, &pos) ||
            !llama_batch_ext_set_output_embd(batch, i, true) ||
            (orders[i] && !llama_batch_ext_set_decision_order(batch, i, (llama_decision_order) orders[i]))) {
            status = -1003;
        }
    }
    if (status == 0) {
        // ggml-vulkan throws (e.g. vk::OutOfDeviceMemoryError while growing its
        // prealloc buffers mid-pass); an exception must not reach Rust.
        try {
            status = llama_process(ctx, LLAMA_PROCESS_TYPE_ENCODE, batch);
        } catch (const std::exception & e) {
            fprintf(stderr, "ln_decide: %s\n", e.what());
            status = -1006;
        } catch (...) {
            status = -1006;
        }
    }
    for (int32_t k = 0; k < n_scores && status == 0; k++) {
        const float * row = llama_get_embeddings_ith(ctx, k);
        if (row) {
            scores[k] = row[0];
        } else {
            status = -1004;
        }
    }
    llama_batch_ext_free(batch);
    llama_free(ctx);
    return status;
}
