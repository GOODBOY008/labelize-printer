// SPDX-License-Identifier: MIT
// PAPPL owns IPP and raster decoding. This adapter only normalizes pages and
// waits for the Rust daemon's durable output receipt before acknowledging a job.
#include <pappl/pappl.h>
#include <cups/raster.h>
#include <errno.h>
#include <math.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#define MAX_PAGES 1000
#define MAX_BYTES (64ULL * 1024 * 1024)
#define MAX_PIXELS (16ULL * 1024 * 1024)

static const char *data_dir, *daemon_path;
static const char *ready_file;
static pid_t parent_pid;
static double default_width = 102, default_height = 152;
static int default_dpmm = 8;
static char default_media[64];

typedef struct {
  char name[32];
  double width_mm, height_mm, dpi_x, dpi_y;
} page_t;

typedef struct {
  char directory[4096];
  char job_id[64];
  page_t pages[MAX_PAGES];
  unsigned count, rows, width, height, bits, expected_pages;
  cups_cspace_t colorspace;
  uint64_t bytes;
  FILE *file;
  const char *error;
} job_t;

static void json_string(FILE *out, const char *value) {
  fputc('"', out);
  for (const unsigned char *p = (const unsigned char *)(value ? value : ""); *p; p++) {
    if (*p == '"' || *p == '\\') { fputc('\\', out); fputc(*p, out); }
    else if (*p < 32) fprintf(out, "\\u%04x", *p);
    else fputc(*p, out);
  }
  fputc('"', out);
}

static bool fail(pappl_job_t *job, job_t *data, const char *message) {
  if (data && !data->error) data->error = message;
  papplLogJob(job, PAPPL_LOGLEVEL_ERROR, "%s", message);
  papplJobSetReasons(job, PAPPL_JREASON_DOCUMENT_FORMAT_ERROR, PAPPL_JREASON_NONE);
  return false;
}

static bool start_job(pappl_job_t *job, pappl_pr_options_t *options, pappl_device_t *device) {
  (void)options; (void)device;
  job_t *data = calloc(1, sizeof(job_t));
  if (!data) return false;
  papplJobSetData(job, data);
  int n = snprintf(data->directory, sizeof(data->directory), "%s/incoming/ipp-%d-XXXXXX", data_dir, papplJobGetID(job));
  if (n < 0 || (size_t)n >= sizeof(data->directory) || !mkdtemp(data->directory))
    return fail(job, data, "Cannot create private raster spool directory");
  int descriptors[2];
  if (pipe(descriptors)) return fail(job, data, "Cannot create daemon control pipe");
  pid_t pid = fork();
  if (!pid) {
    close(descriptors[0]); dup2(descriptors[1], STDOUT_FILENO); close(descriptors[1]);
    execl(daemon_path, daemon_path, "--data-dir", data_dir, "begin-raster", papplJobGetName(job),
          strrchr(data->directory, '/') + 1, (char *)NULL);
    _exit(127);
  }
  close(descriptors[1]);
  ssize_t count = read(descriptors[0], data->job_id, sizeof(data->job_id) - 1);
  close(descriptors[0]);
  int status;
  if (pid < 0 || waitpid(pid, &status, 0) != pid || !WIFEXITED(status) || WEXITSTATUS(status) || count < 36)
    return fail(job, data, "Cannot register raster task with the daemon");
  data->job_id[36] = 0;
  return true;
}

static bool start_page(pappl_job_t *job, pappl_pr_options_t *options, pappl_device_t *device, unsigned page) {
  (void)device; (void)page;
  job_t *data = papplJobGetData(job);
  cups_page_header2_t *h = &options->header;
  if (!data || data->error) return false;
  if (data->count >= MAX_PAGES || !h->cupsWidth || !h->cupsHeight ||
      h->cupsWidth > 32000 || h->cupsHeight > 32000 ||
      (uint64_t)h->cupsWidth * h->cupsHeight > MAX_PIXELS)
    return fail(job, data, "Raster exceeds the page count or canvas limit");
  bool gray = (h->cupsBitsPerPixel == 1 || h->cupsBitsPerPixel == 8) &&
              (h->cupsColorSpace == CUPS_CSPACE_K || h->cupsColorSpace == CUPS_CSPACE_W || h->cupsColorSpace == CUPS_CSPACE_SW);
  bool srgb = h->cupsBitsPerPixel == 24 && h->cupsBitsPerColor == 8 && h->cupsColorSpace == CUPS_CSPACE_SRGB;
  if ((!gray && !srgb) ||
      h->cupsColorOrder != CUPS_ORDER_CHUNKED || !h->HWResolution[0] || !h->HWResolution[1] ||
      h->cupsBytesPerLine < ((uint64_t)h->cupsWidth * h->cupsBitsPerPixel + 7) / 8)
    return fail(job, data, "Unsupported raster pixel format or resolution");
  data->bytes += (uint64_t)h->cupsWidth * h->cupsHeight + 64;
  if (data->bytes > MAX_BYTES) return fail(job, data, "Normalized raster input exceeds 64 MiB");
  data->rows = 0;
  data->width = h->cupsWidth; data->height = h->cupsHeight;
  data->bits = h->cupsBitsPerPixel; data->colorspace = h->cupsColorSpace;
  if (!data->count) data->expected_pages = h->cupsInteger[CUPS_RASTER_PWG_TotalPageCount];
  page_t *p = &data->pages[data->count];
  snprintf(p->name, sizeof(p->name), "page-%04u.pgm", data->count + 1);
  p->dpi_x = h->HWResolution[0]; p->dpi_y = h->HWResolution[1];
  // PWG uses integral point PageSize, Apple raster may only carry pixels/dpi.
  // cupsPageSize is authoritative when present; never scale/rotate the pixels.
  p->width_mm = h->cupsPageSize[0] > 0 ? h->cupsPageSize[0] * 25.4 / 72.0 : h->cupsWidth * 25.4 / p->dpi_x;
  p->height_mm = h->cupsPageSize[1] > 0 ? h->cupsPageSize[1] * 25.4 / 72.0 : h->cupsHeight * 25.4 / p->dpi_y;
  char filename[4160];
  snprintf(filename, sizeof(filename), "%s/%s", data->directory, p->name);
  data->file = fopen(filename, "wb");
  if (!data->file || fprintf(data->file, "P5\n%u %u\n255\n", data->width, data->height) < 0)
    return fail(job, data, "Cannot write raster page");
  return true;
}

static bool write_line(pappl_job_t *job, pappl_pr_options_t *options, pappl_device_t *device, unsigned y, const unsigned char *line) {
  (void)options; (void)device;
  job_t *data = papplJobGetData(job);
  if (!data || data->error) return false;
  if (!data->file || y != data->rows || y >= data->height) return fail(job, data, "Unexpected raster row order");
  unsigned char pixels[32000];
  if (data->colorspace == CUPS_CSPACE_SRGB) {
    for (unsigned x = 0; x < data->width; x++) {
      const unsigned char *rgb = line + 3 * x;
      pixels[x] = (299u * rgb[0] + 587u * rgb[1] + 114u * rgb[2] + 500u) / 1000u;
    }
  } else {
    for (unsigned x = 0; x < data->width; x++) {
      unsigned char value = data->bits == 8 ? line[x] : ((line[x / 8] & (128 >> (x % 8))) ? 255 : 0);
      pixels[x] = data->colorspace == CUPS_CSPACE_K ? 255 - value : value;
    }
  }
  if (fwrite(pixels, 1, data->width, data->file) != data->width) return fail(job, data, "Raster spool write failed (check free disk space)");
  data->rows++;
  return true;
}

static bool end_page(pappl_job_t *job, pappl_pr_options_t *options, pappl_device_t *device, unsigned page) {
  (void)options; (void)device; (void)page;
  job_t *data = papplJobGetData(job);
  if (!data) return false;
  bool complete = data->file && data->rows == data->height && !data->error;
  if (data->file) {
    if (fflush(data->file) || fsync(fileno(data->file))) complete = false;
    if (fclose(data->file)) complete = false;
    data->file = NULL;
  }
  if (!complete) return fail(job, data, "Incomplete raster page or spool write failure");
  data->count++;
  return true;
}

static bool end_job(pappl_job_t *job, pappl_pr_options_t *options, pappl_device_t *device) {
  (void)device;
  job_t *data = papplJobGetData(job);
  if (!data) return false;
  if (data->file) { fclose(data->file); data->file = NULL; }
  if (papplJobIsCanceled(job) || papplJobGetState(job) == IPP_JSTATE_ABORTED)
    data->error = data->error ? data->error : "IPP task was canceled or interrupted";
  if (!data->count || (data->expected_pages && data->expected_pages != data->count))
    data->error = data->error ? data->error : "Raster page count does not match the document";
  int copies = options->copies > 0 ? options->copies : 1;
  int per_set = ippGetInteger(papplJobGetAttribute(job, "job-pages-per-set"), 0);
  // IPP clients that set job-pages-per-set have already expanded copies in the
  // document. Without it, expand the IPP copies attribute exactly once here.
  if (per_set > 0) {
    if ((uint64_t)per_set * copies != data->count) data->error = "Incomplete pre-expanded IPP copies";
    copies = 1;
  }
  if ((uint64_t)data->count * copies > MAX_PAGES || data->bytes * copies > MAX_BYTES)
    data->error = "Expanded raster job exceeds 1000 pages or 64 MiB";
  char manifest[4160];
  snprintf(manifest, sizeof(manifest), "%s/manifest.json", data->directory);
  FILE *out = fopen(manifest, "wb");
  bool ok = false;
  if (out) {
    fprintf(out, "{\"job_id\":"); json_string(out, data->job_id);
    fprintf(out, ",\"title\":"); json_string(out, papplJobGetName(job));
    fprintf(out, ",\"source_job_id\":"); json_string(out, strrchr(data->directory, '/') + 1);
    fprintf(out, ",\"error\":"); if (data->error) json_string(out, data->error); else fprintf(out, "null");
    fprintf(out, ",\"pages\":[");
    const char *handling = ippGetString(papplJobGetAttribute(job, "multiple-document-handling"), 0, NULL);
    bool uncollated = handling && !strcmp(handling, "separate-documents-uncollated-copies");
    if (!data->error) for (unsigned i = 0; i < data->count * (unsigned)copies; i++) {
      page_t *p = &data->pages[uncollated ? i / copies : i % data->count];
      fprintf(out, "%s{\"file\":", i ? "," : ""); json_string(out, p->name);
      fprintf(out, ",\"width_mm\":%.9g,\"height_mm\":%.9g,\"dpi_x\":%.9g,\"dpi_y\":%.9g}",
              p->width_mm, p->height_mm, p->dpi_x, p->dpi_y);
    }
    fprintf(out, "]}\n");
    ok = !ferror(out) && !fflush(out) && !fsync(fileno(out));
    if (fclose(out)) ok = false;
  }
  if (ok) {
    pid_t pid = fork();
    if (!pid) {
      execl(daemon_path, daemon_path, "--data-dir", data_dir, "ingest-raster", manifest, "--wait", (char *)NULL);
      _exit(127);
    }
    int status = 0;
    if (pid < 0) ok = false;
    else {
      pid_t waited;
      do { waited = waitpid(pid, &status, 0); } while (waited < 0 && errno == EINTR);
      ok = waited == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0;
    }
  }
  if (!ok) {
    papplLogJob(job, PAPPL_LOGLEVEL_ERROR, "Output was not saved; inspect Labelize Printer task history and %s", data->directory);
    // A full spool disk can prevent even the manifest from being written.
    // Report over IPC so a registered task does not remain Receiving forever.
    if (data->job_id[0]) {
      char message[4352];
      snprintf(message, sizeof(message), "%s; retained input: %s",
               data->error ? data->error : "Raster submission failed; check free disk space and daemon log", data->directory);
      pid_t pid = fork();
      if (!pid) {
        execl(daemon_path, daemon_path, "--data-dir", data_dir, "abort-raster", data->job_id, message, (char *)NULL);
        _exit(127);
      }
      if (pid > 0) { while (waitpid(pid, NULL, 0) < 0 && errno == EINTR) {} }
    }
  }
  free(data);
  papplJobSetData(job, NULL);
  return ok;
}

static bool status_cb(pappl_printer_t *printer) { (void)printer; return true; }
static bool print_file(pappl_job_t *job, pappl_pr_options_t *options, pappl_device_t *device) {
  (void)options; (void)device;
  return fail(job, NULL, "Submit PWG/Apple raster through IPP, or ZPL through the RAW port");
}

static bool driver_cb(pappl_system_t *system, const char *name, const char *uri, const char *id,
                      pappl_pr_driver_data_t *data, ipp_t **attrs, void *context) {
  (void)system; (void)name; (void)uri; (void)id; (void)attrs; (void)context;
  data->format = "image/pwg-raster";
  data->rstartjob_cb = start_job; data->rstartpage_cb = start_page;
  data->rwriteline_cb = write_line; data->rendpage_cb = end_page; data->rendjob_cb = end_job;
  data->printfile_cb = print_file; data->status_cb = status_cb;
  snprintf(data->make_and_model, sizeof(data->make_and_model), "Labelize Printer");
  data->kind = PAPPL_KIND_LABEL | PAPPL_KIND_DOCUMENT;
  data->ppm = 20;
  data->raster_types = PAPPL_PWG_RASTER_TYPE_SGRAY_8 | PAPPL_PWG_RASTER_TYPE_BLACK_8 | PAPPL_PWG_RASTER_TYPE_BLACK_1;
  data->force_raster_type = PAPPL_PWG_RASTER_TYPE_SGRAY_8;
  data->color_supported = PAPPL_COLOR_MODE_MONOCHROME;
  data->color_default = PAPPL_COLOR_MODE_MONOCHROME;
  data->num_resolution = 2;
  data->x_resolution[0] = data->y_resolution[0] = 203;
  data->x_resolution[1] = data->y_resolution[1] = 300;
  data->x_default = data->y_default = default_dpmm == 12 ? 300 : 203;
  data->orient_default = IPP_ORIENT_NONE;
  data->quality_default = IPP_QUALITY_NORMAL;
  data->sides_supported = data->sides_default = PAPPL_SIDES_ONE_SIDED;
  data->borderless = true;
  const char *media[] = { default_media, "na_index-4x6_4x6in", "iso_a4_210x297mm", "na_letter_8.5x11in",
    "oe_label_50x30mm", "oe_label_100x150mm", "custom_min_10x10mm", "custom_max_320x500mm" };
  data->num_media = (int)(sizeof(media) / sizeof(media[0]));
  memcpy(data->media, media, sizeof(media));
  data->num_source = 1; data->source[0] = "main";
  data->num_type = 1; data->type[0] = "labels";
  snprintf(data->media_default.size_name, sizeof(data->media_default.size_name), "%s", default_media);
  data->media_default.size_width = (int)lround(default_width * 100);
  data->media_default.size_length = (int)lround(default_height * 100);
  snprintf(data->media_default.source, sizeof(data->media_default.source), "main");
  snprintf(data->media_default.type, sizeof(data->media_default.type), "labels");
  data->media_ready[0] = data->media_default;
  return true;
}

static bool monitor_parent(pappl_system_t *system, void *data) {
  (void)data;
  if (parent_pid > 1 && kill(parent_pid, 0) && errno == ESRCH) { papplSystemShutdown(system); return false; }
  return true;
}

int main(int argc, char **argv) {
  if (argc == 2 && !strcmp(argv[1], "--version")) { printf("labelize-printer-ipp %s\n", LABELIZE_PRINTER_VERSION); return 0; }
  int port = 8631;
  umask(0077);
  for (int i = 1; i + 1 < argc; i += 2) {
    if (!strcmp(argv[i], "--port")) port = atoi(argv[i + 1]);
    else if (!strcmp(argv[i], "--data-dir")) data_dir = argv[i + 1];
    else if (!strcmp(argv[i], "--daemon")) daemon_path = argv[i + 1];
    else if (!strcmp(argv[i], "--ready-file")) ready_file = argv[i + 1];
    else if (!strcmp(argv[i], "--parent-pid")) parent_pid = (pid_t)atoi(argv[i + 1]);
    else if (!strcmp(argv[i], "--width-mm")) default_width = atof(argv[i + 1]);
    else if (!strcmp(argv[i], "--height-mm")) default_height = atof(argv[i + 1]);
    else if (!strcmp(argv[i], "--dpmm")) default_dpmm = atoi(argv[i + 1]);
    else { fprintf(stderr, "Unknown argument: %s\n", argv[i]); return 2; }
  }
  if (!data_dir || !daemon_path || port < 1024 || port > 65535) return 2;
  snprintf(default_media, sizeof(default_media), "oe_label_%.2fx%.2fmm", default_width, default_height);
  char spool[4096];
  snprintf(spool, sizeof(spool), "%s/ipp-spool", data_dir);
  if (mkdir(spool, 0700) && errno != EEXIST) return 1;
  pappl_system_t *system = papplSystemCreate(PAPPL_SOPTIONS_NO_TLS, "Labelize Printer", port, NULL, spool, "-", PAPPL_LOGLEVEL_INFO, NULL, false);
  if (!system) return 1;
  papplSystemSetHostName(system, "localhost");
  papplSystemSetDNSSDName(system, NULL);
  papplSystemSetMaxClients(system, 8);
  papplSystemSetMaxImageSize(system, MAX_BYTES, 32000, 32000);
  if (!papplSystemAddListeners(system, "127.0.0.1")) return 1;
  static pappl_pr_driver_t drivers[] = { { "labelize", "Labelize Printer", "MFG:Labelize;MDL:Printer;CMD:PWGRaster;", NULL } };
  papplSystemSetPrinterDrivers(system, 1, drivers, NULL, NULL, driver_cb, NULL);
  pappl_printer_t *printer = papplPrinterCreate(system, 1, "Labelize Printer", "labelize", NULL, "file:///dev/null");
  if (!printer) return 1;
  papplPrinterSetDNSSDName(printer, NULL);
  papplPrinterSetMaxActiveJobs(printer, 16);
  papplPrinterSetMaxCompletedJobs(printer, 100);
  papplPrinterSetMaxPreservedJobs(printer, 0);
  papplSystemAddTimerCallback(system, 0, 2, monitor_parent, NULL);
  if (ready_file) {
    FILE *ready = fopen(ready_file, "w");
    if (!ready) return 1;
    fprintf(ready, "%ld\n", (long)getpid());
    if (fclose(ready)) return 1;
  }
  papplSystemRun(system);
  papplSystemDelete(system);
  return 0;
}
