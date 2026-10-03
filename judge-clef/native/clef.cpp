// The slice of llama.cpp judge-clef calls, as a C API of scalars and pointers
// (src/llama.rs declares it by hand): llama.h passes structs by value, and
// llama_batch_ext_set_decision_order is C++ (src/llama-ext.h).
#include "ggml-backend.h"
#include "llama.h"
#include "llama-ext.h"

#include <cstdio>
#include <cstdlib>
#include <strings.h>

// llama.cpp logs every tensor while loading (~1300 lines): keep warnings and
// errors. A continuation line follows its message's fate.
static void log_warnings(ggml_log_level level, const char * text, void *) {
    thread_local bool keep = false;
    if (level != GGML_LOG_LEVEL_CONT) {
        keep = level >= GGML_LOG_LEVEL_WARN;
    }
    if (keep) {
        fputs(text, stderr);
    }
}

// Load the backend modules (GGML_BACKEND_DL builds) from `dir`, else from
// `fallback` when `dir` had none; both may be null.
extern "C" void clef_backend_init(const char * dir, const char * fallback) {
    // A context per call starts ggml-vulkan's submit sizing from zero flops,
    // so each pass submits its graph in 100-node pieces; at 16k tokens one
    // nears amdgpu's 2 s job timeout, which loses the device and aborts the
    // process. 10-node pieces cost at most 0.5% (RX 6900 XT). An operator's
    // value wins.
    setenv("GGML_VK_MAX_NODES_PER_SUBMIT", "10", 0);
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
extern "C" bool clef_gpu_device(char * out, size_t len) {
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

// gpu_layers: -1 every layer, n that many; 0 uses no GPU device at all (with
// one, llama.cpp keeps CPU weights in its pinned host buffer, unrepacked).
extern "C" llama_model * clef_model_load(const char * path, int32_t gpu_layers) {
    llama_model_params params = llama_model_default_params();
    ggml_backend_dev_t no_devices[] = { nullptr };
    params.n_gpu_layers = gpu_layers;
    if (gpu_layers == 0) {
        params.devices = no_devices;
    }
    return llama_model_load_from_file(path, params);
}

// One forward of Clef over the whole prompt in a context sized to it, freed
// before returning: n_ctx = n_batch = n_ubatch = n_tokens rounded up to 256,
// the padding llama.cpp gives n_ctx anyway (it aborts the process on an
// encode longer than n_ubatch). orders[i] is the token's
// llama_decision_order (0 none, 1-3 question noul/choice/score, 4 option);
// scores[k] gets option k's score, options in prompt order.
// Returns 0, -1001 on bad input, -1002 when the context cannot be created,
// -1003 when the batch refuses a token, -1004 with no score, else
// llama_process's own code.
extern "C" int32_t clef_decide(const llama_model * model, int32_t n_threads, const int32_t * ids,
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
        status = llama_process(ctx, LLAMA_PROCESS_TYPE_ENCODE, batch);
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
