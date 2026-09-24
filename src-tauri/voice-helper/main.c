// Bridge's isolated sherpa-onnx streaming helper.
//
// The helper is deliberately tiny: it loads the explicitly installed,
// checksum-pinned sherpa-onnx 1.13.8 runtime at run time, owns one recognizer
// and one take, and speaks a bounded binary protocol over stdin/stdout. It
// never opens a microphone, network connection, credential store, or log file.
// Dropping/killing the process releases every native inference allocation.

#include <dlfcn.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Minimal ABI declarations copied from sherpa-onnx v1.13.8's public C header.
// Keep these layouts pinned with the runtime version in voice/sherpa.rs.
typedef struct {
  const char *encoder;
  const char *decoder;
  const char *joiner;
} SherpaOnnxOnlineTransducerModelConfig;

typedef struct {
  const char *encoder;
  const char *decoder;
} SherpaOnnxOnlineParaformerModelConfig;

typedef struct {
  const char *model;
} SherpaOnnxOnlineZipformer2CtcModelConfig;

typedef struct {
  const char *model;
} SherpaOnnxOnlineNemoCtcModelConfig;

typedef struct {
  const char *model;
} SherpaOnnxOnlineToneCtcModelConfig;

typedef struct {
  SherpaOnnxOnlineTransducerModelConfig transducer;
  SherpaOnnxOnlineParaformerModelConfig paraformer;
  SherpaOnnxOnlineZipformer2CtcModelConfig zipformer2_ctc;
  const char *tokens;
  int32_t num_threads;
  const char *provider;
  int32_t debug;
  const char *model_type;
  const char *modeling_unit;
  const char *bpe_vocab;
  const char *tokens_buf;
  int32_t tokens_buf_size;
  SherpaOnnxOnlineNemoCtcModelConfig nemo_ctc;
  SherpaOnnxOnlineToneCtcModelConfig t_one_ctc;
} SherpaOnnxOnlineModelConfig;

typedef struct {
  int32_t sample_rate;
  int32_t feature_dim;
} SherpaOnnxFeatureConfig;

typedef struct {
  const char *graph;
  int32_t max_active;
} SherpaOnnxOnlineCtcFstDecoderConfig;

typedef struct {
  const char *dict_dir;
  const char *lexicon;
  const char *rule_fsts;
} SherpaOnnxHomophoneReplacerConfig;

typedef struct {
  SherpaOnnxFeatureConfig feat_config;
  SherpaOnnxOnlineModelConfig model_config;
  const char *decoding_method;
  int32_t max_active_paths;
  int32_t enable_endpoint;
  float rule1_min_trailing_silence;
  float rule2_min_trailing_silence;
  float rule3_min_utterance_length;
  const char *hotwords_file;
  float hotwords_score;
  SherpaOnnxOnlineCtcFstDecoderConfig ctc_fst_decoder_config;
  const char *rule_fsts;
  const char *rule_fars;
  float blank_penalty;
  const char *hotwords_buf;
  int32_t hotwords_buf_size;
  SherpaOnnxHomophoneReplacerConfig hr;
} SherpaOnnxOnlineRecognizerConfig;

typedef struct SherpaOnnxOnlineRecognizer SherpaOnnxOnlineRecognizer;
typedef struct SherpaOnnxOnlineStream SherpaOnnxOnlineStream;

typedef struct {
  const char *text;
  const char *tokens;
  const char *const *tokens_arr;
  float *timestamps;
  int32_t count;
  const char *json;
} SherpaOnnxOnlineRecognizerResult;

typedef struct {
  const SherpaOnnxOnlineRecognizer *(*create_recognizer)(
      const SherpaOnnxOnlineRecognizerConfig *);
  void (*destroy_recognizer)(const SherpaOnnxOnlineRecognizer *);
  const SherpaOnnxOnlineStream *(*create_stream)(
      const SherpaOnnxOnlineRecognizer *);
  void (*destroy_stream)(const SherpaOnnxOnlineStream *);
  void (*accept_waveform)(const SherpaOnnxOnlineStream *, int32_t,
                          const float *, int32_t);
  int32_t (*is_ready)(const SherpaOnnxOnlineRecognizer *,
                      const SherpaOnnxOnlineStream *);
  void (*decode)(const SherpaOnnxOnlineRecognizer *,
                 const SherpaOnnxOnlineStream *);
  const SherpaOnnxOnlineRecognizerResult *(*get_result)(
      const SherpaOnnxOnlineRecognizer *, const SherpaOnnxOnlineStream *);
  void (*destroy_result)(const SherpaOnnxOnlineRecognizerResult *);
  void (*input_finished)(const SherpaOnnxOnlineStream *);
} SherpaApi;

enum {
  COMMAND_APPEND = 1,
  COMMAND_FINISH = 2,
  COMMAND_CANCEL = 3,
  RESPONSE_READY = 0x80,
  RESPONSE_PARTIAL = 0x81,
  RESPONSE_FINAL = 0x82,
  RESPONSE_CLOSED = 0x83,
  RESPONSE_ERROR = 0xff,
  MAX_AUDIO_BYTES = 64 * 1024,
  MAX_TEXT_BYTES = 64 * 1024,
};

static int write_all(const void *bytes, size_t length) {
  const unsigned char *cursor = bytes;
  while (length > 0) {
    size_t written = fwrite(cursor, 1, length, stdout);
    if (written == 0) return 0;
    cursor += written;
    length -= written;
  }
  return 1;
}

static int read_all(void *bytes, size_t length) {
  unsigned char *cursor = bytes;
  while (length > 0) {
    size_t read = fread(cursor, 1, length, stdin);
    if (read == 0) return 0;
    cursor += read;
    length -= read;
  }
  return 1;
}

static int write_frame(uint8_t kind, const char *payload, uint32_t length) {
  unsigned char header[5] = {kind, (unsigned char)(length & 0xff),
                             (unsigned char)((length >> 8) & 0xff),
                             (unsigned char)((length >> 16) & 0xff),
                             (unsigned char)((length >> 24) & 0xff)};
  if (!write_all(header, sizeof header) ||
      (length > 0 && !write_all(payload, length))) {
    return 0;
  }
  return fflush(stdout) == 0;
}

static int fail(const char *code) {
  size_t length = strlen(code);
  if (length > MAX_TEXT_BYTES) length = MAX_TEXT_BYTES;
  (void)write_frame(RESPONSE_ERROR, code, (uint32_t)length);
  return 1;
}

static int path_join(char *out, size_t size, const char *root,
                     const char *suffix) {
  int n = snprintf(out, size, "%s/%s", root, suffix);
  return n >= 0 && (size_t)n < size;
}

static size_t bounded_length(const char *text, size_t limit) {
  if (!text) return 0;
  size_t length = 0;
  while (length <= limit && text[length]) ++length;
  return length;
}

static int load_symbol(void *library, const char *name, void *target,
                       size_t target_size) {
  void *symbol = dlsym(library, name);
  if (!symbol || target_size != sizeof symbol) return 0;
  memcpy(target, &symbol, sizeof symbol);
  return 1;
}

#define LOAD(api, library, field, symbol)                                      \
  load_symbol((library), (symbol), &(api).field, sizeof((api).field))

static int load_api(void *library, SherpaApi *api) {
  memset(api, 0, sizeof *api);
  return LOAD(*api, library, create_recognizer,
              "SherpaOnnxCreateOnlineRecognizer") &&
         LOAD(*api, library, destroy_recognizer,
              "SherpaOnnxDestroyOnlineRecognizer") &&
         LOAD(*api, library, create_stream, "SherpaOnnxCreateOnlineStream") &&
         LOAD(*api, library, destroy_stream,
              "SherpaOnnxDestroyOnlineStream") &&
         LOAD(*api, library, accept_waveform,
              "SherpaOnnxOnlineStreamAcceptWaveform") &&
         LOAD(*api, library, is_ready, "SherpaOnnxIsOnlineStreamReady") &&
         LOAD(*api, library, decode, "SherpaOnnxDecodeOnlineStream") &&
         LOAD(*api, library, get_result,
              "SherpaOnnxGetOnlineStreamResult") &&
         LOAD(*api, library, destroy_result,
              "SherpaOnnxDestroyOnlineRecognizerResult") &&
         LOAD(*api, library, input_finished,
              "SherpaOnnxOnlineStreamInputFinished");
}

static int emit_result(const SherpaApi *api,
                       const SherpaOnnxOnlineRecognizer *recognizer,
                       const SherpaOnnxOnlineStream *stream, uint8_t kind) {
  const SherpaOnnxOnlineRecognizerResult *result =
      api->get_result(recognizer, stream);
  if (!result) return 0;
  size_t length = bounded_length(result->text, MAX_TEXT_BYTES);
  int ok = length <= MAX_TEXT_BYTES &&
           write_frame(kind, result->text ? result->text : "",
                       (uint32_t)length);
  api->destroy_result(result);
  return ok;
}

int main(int argc, char **argv) {
  if (argc != 3) {
    fprintf(stderr, "usage: bridge-voice-helper RUNTIME_DIR MODEL_DIR\n");
    return 2;
  }

  char library_path[4096], encoder[4096], decoder[4096], joiner[4096],
      tokens[4096];
  if (!path_join(library_path, sizeof library_path, argv[1],
                 "lib/libsherpa-onnx-c-api.dylib") ||
      !path_join(encoder, sizeof encoder, argv[2], "encoder.int8.onnx") ||
      !path_join(decoder, sizeof decoder, argv[2], "decoder.int8.onnx") ||
      !path_join(joiner, sizeof joiner, argv[2], "joiner.int8.onnx") ||
      !path_join(tokens, sizeof tokens, argv[2], "tokens.txt")) {
    return fail("path_too_long");
  }

  void *library = dlopen(library_path, RTLD_NOW | RTLD_LOCAL);
  if (!library) return fail("runtime_unavailable");
  SherpaApi api;
  if (!load_api(library, &api)) {
    dlclose(library);
    return fail("runtime_incompatible");
  }

  SherpaOnnxOnlineRecognizerConfig config;
  memset(&config, 0, sizeof config);
  config.feat_config.sample_rate = 16000;
  config.feat_config.feature_dim = 80;
  config.model_config.transducer.encoder = encoder;
  config.model_config.transducer.decoder = decoder;
  config.model_config.transducer.joiner = joiner;
  config.model_config.tokens = tokens;
  config.model_config.num_threads = 1;
  config.model_config.provider = "cpu";
  config.decoding_method = "greedy_search";
  config.enable_endpoint = 0;

  const SherpaOnnxOnlineRecognizer *recognizer =
      api.create_recognizer(&config);
  if (!recognizer) {
    dlclose(library);
    return fail("model_load_failed");
  }
  const SherpaOnnxOnlineStream *stream = api.create_stream(recognizer);
  if (!stream) {
    api.destroy_recognizer(recognizer);
    dlclose(library);
    return fail("stream_start_failed");
  }
  if (!write_frame(RESPONSE_READY, NULL, 0)) goto io_error;

  for (;;) {
    unsigned char header[5];
    if (!read_all(header, sizeof header)) goto io_error;
    uint32_t length = (uint32_t)header[1] | ((uint32_t)header[2] << 8) |
                      ((uint32_t)header[3] << 16) |
                      ((uint32_t)header[4] << 24);
    if (header[0] == COMMAND_APPEND) {
      if (length == 0 || length > MAX_AUDIO_BYTES || (length & 1) != 0) {
        fail("invalid_audio_frame");
        break;
      }
      unsigned char *pcm = malloc(length);
      float *samples = malloc((length / 2) * sizeof(float));
      if (!pcm || !samples || !read_all(pcm, length)) {
        free(pcm);
        free(samples);
        goto io_error;
      }
      for (uint32_t i = 0; i < length / 2; ++i) {
        uint16_t bits = (uint16_t)pcm[i * 2] | ((uint16_t)pcm[i * 2 + 1] << 8);
        samples[i] = (float)(int16_t)bits / 32768.0f;
      }
      api.accept_waveform(stream, 16000, samples, (int32_t)(length / 2));
      free(samples);
      free(pcm);
      while (api.is_ready(recognizer, stream)) api.decode(recognizer, stream);
      if (!emit_result(&api, recognizer, stream, RESPONSE_PARTIAL))
        goto io_error;
    } else if (header[0] == COMMAND_FINISH) {
      if (length != 0) {
        fail("invalid_finish_frame");
        break;
      }
      float *tail = calloc(16000, sizeof(float));
      if (!tail) goto io_error;
      api.accept_waveform(stream, 16000, tail, 16000);
      free(tail);
      api.input_finished(stream);
      while (api.is_ready(recognizer, stream)) api.decode(recognizer, stream);
      if (!emit_result(&api, recognizer, stream, RESPONSE_FINAL) ||
          !write_frame(RESPONSE_CLOSED, NULL, 0))
        goto io_error;
      api.destroy_stream(stream);
      api.destroy_recognizer(recognizer);
      dlclose(library);
      return 0;
    } else if (header[0] == COMMAND_CANCEL) {
      if (length != 0) {
        fail("invalid_cancel_frame");
        break;
      }
      (void)write_frame(RESPONSE_CLOSED, NULL, 0);
      api.destroy_stream(stream);
      api.destroy_recognizer(recognizer);
      dlclose(library);
      return 0;
    } else {
      fail("unknown_command");
      break;
    }
  }

  api.destroy_stream(stream);
  api.destroy_recognizer(recognizer);
  dlclose(library);
  return 1;

io_error:
  api.destroy_stream(stream);
  api.destroy_recognizer(recognizer);
  dlclose(library);
  return errno == 0 ? 1 : errno;
}
