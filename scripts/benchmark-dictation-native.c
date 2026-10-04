// Development-only sherpa-onnx C ABI spike; never opens a microphone or socket.
// Build/run instructions and pinned artifacts: docs/plans/voice-engine-evaluation.md.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <time.h>

#include "sherpa-onnx/c-api/c-api.h"

static double now_ms(void) {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return ts.tv_sec * 1000.0 + ts.tv_nsec / 1000000.0;
}

static double cpu_ms(void) {
  struct rusage usage;
  getrusage(RUSAGE_SELF, &usage);
  return (usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) * 1000.0 +
         (usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) / 1000.0;
}

static double peak_rss_mib(void) {
  struct rusage usage;
  getrusage(RUSAGE_SELF, &usage);
#ifdef __APPLE__
  return usage.ru_maxrss / 1048576.0;
#else
  return usage.ru_maxrss / 1024.0;
#endif
}

static void json_string(const char *text) {
  putchar('"');
  for (const unsigned char *p = (const unsigned char *)text; *p; ++p) {
    if (*p == '"' || *p == '\\') printf("\\%c", *p);
    else if (*p < 32) printf("\\u%04x", *p);
    else putchar(*p);
  }
  putchar('"');
}

static int model_path(char *out, size_t size, const char *root, const char *file) {
  int n = snprintf(out, size, "%s/%s", root, file);
  return n >= 0 && (size_t)n < size;
}

static int run_take(const SherpaOnnxOnlineRecognizer *recognizer,
                    const float *samples, int count, int fixture, int repeat,
                    int print_text) {
  double begin = now_ms(), cpu_begin = cpu_ms();
  const SherpaOnnxOnlineStream *stream = SherpaOnnxCreateOnlineStream(recognizer);
  if (!stream) return 0;
  double first_partial_input_ms = -1, max_chunk_ms = 0;
  // Feed 100 ms blocks without sleeping: this measures decoding throughput,
  // NOT microphone-to-screen latency. Retain the final short block.
  for (int offset = 0; offset < count; offset += 1600) {
    int n = count - offset < 1600 ? count - offset : 1600;
    double chunk_begin = now_ms();
    SherpaOnnxOnlineStreamAcceptWaveform(stream, 16000, samples + offset, n);
    while (SherpaOnnxIsOnlineStreamReady(recognizer, stream)) {
      SherpaOnnxDecodeOnlineStream(recognizer, stream);
    }
    double chunk_ms = now_ms() - chunk_begin;
    if (chunk_ms > max_chunk_ms) max_chunk_ms = chunk_ms;
    if (first_partial_input_ms < 0) {
      const SherpaOnnxOnlineRecognizerResult *partial =
          SherpaOnnxGetOnlineStreamResult(recognizer, stream);
      if (!partial) { SherpaOnnxDestroyOnlineStream(stream); return 0; }
      if (partial->text && partial->text[0])
        first_partial_input_ms = (offset + n) / 16.0;
      SherpaOnnxDestroyOnlineRecognizerResult(partial);
    }
  }

  double flush_begin = now_ms();
  // Match Omnigent's explicit one-second tail; this padding is not user audio
  // and must not be counted in the real-time-factor denominator.
  const float tail[16000] = {0};
  SherpaOnnxOnlineStreamAcceptWaveform(stream, 16000, tail, 16000);
  SherpaOnnxOnlineStreamInputFinished(stream);
  while (SherpaOnnxIsOnlineStreamReady(recognizer, stream)) {
    SherpaOnnxDecodeOnlineStream(recognizer, stream);
  }
  const SherpaOnnxOnlineRecognizerResult *result =
      SherpaOnnxGetOnlineStreamResult(recognizer, stream);
  if (!result) { SherpaOnnxDestroyOnlineStream(stream); return 0; }
  double end = now_ms(), cpu = cpu_ms() - cpu_begin;
  const char *text = result->text ? result->text : "";
  printf("{\"event\":\"take\",\"fixture\":%d,\"repeat\":%d,"
         "\"audio_ms\":%.3f,\"decode_ms\":%.3f,\"cpu_ms\":%.3f,"
         "\"real_time_factor\":%.4f,\"flush_ms\":%.3f,\"max_chunk_ms\":%.3f,"
         "\"first_partial_input_ms\":",
         fixture, repeat, count / 16.0, end - begin, cpu,
         (end - begin) / (count / 16.0), end - flush_begin, max_chunk_ms);
  if (first_partial_input_ms < 0) printf("null");
  else printf("%.3f", first_partial_input_ms);
  printf(",\"peak_process_rss_mib\":%.3f,\"text_bytes\":%zu",
         peak_rss_mib(), strlen(text));
  // Opt in only for consented/public fixtures. No transcript in default output.
  if (print_text) { printf(",\"text\":"); json_string(text); }
  printf("}\n");
  fflush(stdout);
  SherpaOnnxDestroyOnlineRecognizerResult(result);
  SherpaOnnxDestroyOnlineStream(stream);
  return 1;
}

int main(int argc, char **argv) {
  if (argc < 4) {
    fprintf(stderr, "Usage: %s MODEL_DIR THREADS [--print-text] WAV_OR_--silence...\n",
            argv[0]);
    return 2;
  }
  char *end;
  long threads = strtol(argv[2], &end, 10);
  if (!argv[2][0] || *end || threads < 1 || threads > 8) {
    fprintf(stderr, "THREADS must be 1..8\n");
    return 2;
  }
  int print_text = strcmp(argv[3], "--print-text") == 0;
  int first = print_text ? 4 : 3;
  if (first == argc) return 2;
  char encoder[4096], decoder[4096], joiner[4096], tokens[4096];
  if (!model_path(encoder, sizeof encoder, argv[1], "encoder.int8.onnx") ||
      !model_path(decoder, sizeof decoder, argv[1], "decoder.int8.onnx") ||
      !model_path(joiner, sizeof joiner, argv[1], "joiner.int8.onnx") ||
      !model_path(tokens, sizeof tokens, argv[1], "tokens.txt")) return 2;

  SherpaOnnxOnlineRecognizerConfig config = {0};
  config.feat_config.sample_rate = 16000;
  config.feat_config.feature_dim = 80;
  config.model_config.transducer.encoder = encoder;
  config.model_config.transducer.decoder = decoder;
  config.model_config.transducer.joiner = joiner;
  config.model_config.tokens = tokens;
  config.model_config.num_threads = (int)threads;
  config.model_config.provider = "cpu";
  config.decoding_method = "greedy_search";
  // Isolate take finalization. Endpoint segmentation is a separate app test.
  config.enable_endpoint = 0;
  double begin = now_ms();
  const SherpaOnnxOnlineRecognizer *recognizer =
      SherpaOnnxCreateOnlineRecognizer(&config);
  if (!recognizer) { fprintf(stderr, "Model load failed\n"); return 1; }
  printf("{\"event\":\"load\",\"threads\":%ld,\"model_load_ms\":%.3f,"
         "\"peak_process_rss_mib\":%.3f}\n",
         threads, now_ms() - begin, peak_rss_mib());
  fflush(stdout);

  int status = 0;
  for (int i = first; i < argc; ++i) {
    const SherpaOnnxWave *wave = NULL;
    float *silence = NULL;
    const float *samples;
    int count;
    if (strcmp(argv[i], "--silence") == 0) {
      count = 16000 * 10;
      silence = calloc((size_t)count, sizeof(float));
      if (!silence) { status = 1; break; }
      samples = silence;
    } else {
      wave = SherpaOnnxReadWave(argv[i]);
      if (!wave || wave->sample_rate != 16000 || wave->num_samples <= 0 ||
          wave->num_samples > 16000 * 120) {
        fprintf(stderr, "Fixture %d must be a nonempty 16 kHz WAV, <=120 seconds\n",
                i - first);
        if (wave) SherpaOnnxFreeWave(wave);
        status = 1;
        break;
      }
      samples = wave->samples;
      count = wave->num_samples;
    }
    // The first stream and subsequent streams share one loaded recognizer.
    for (int repeat = 0; repeat < 3; ++repeat) {
      if (!run_take(recognizer, samples, count, i - first, repeat, print_text)) {
        fprintf(stderr, "Inference failed\n"); status = 1; break;
      }
    }
    free(silence);
    if (wave) SherpaOnnxFreeWave(wave);
    if (status) break;
  }
  SherpaOnnxDestroyOnlineRecognizer(recognizer);
  return status;
}
