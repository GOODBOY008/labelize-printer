// Synthetic, asymmetric two-page PWG/Apple raster fixture for protocol tests.
#include <cups/raster.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
int main(int argc, char **argv) {
  if (argc != 4) return 2;
  int fd = open(argv[1], O_CREAT | O_TRUNC | O_WRONLY, 0600);
  cups_raster_t *r = cupsRasterOpen(fd, !strcmp(argv[2], "urf") ? CUPS_RASTER_WRITE_APPLE : CUPS_RASTER_WRITE_PWG);
  int bits = atoi(argv[3]);
  for (int p = 0; p < 2; p++) {
    cups_page_header2_t h; memset(&h, 0, sizeof(h));
    h.cupsWidth = p ? 406 : 812; h.cupsHeight = p ? 609 : 1218;
    h.HWResolution[0] = h.HWResolution[1] = 203;
    h.PageSize[0] = h.cupsPageSize[0] = p ? 144 : 288;
    h.PageSize[1] = h.cupsPageSize[1] = p ? 216 : 432;
    h.cupsBitsPerPixel = bits;
    h.cupsBitsPerColor = bits == 24 ? 8 : bits;
    h.cupsBytesPerLine = (h.cupsWidth * bits + 7) / 8;
    h.cupsColorOrder = CUPS_ORDER_CHUNKED;
    h.cupsColorSpace = bits == 24 ? CUPS_CSPACE_SRGB : (bits == 1 ? CUPS_CSPACE_K : CUPS_CSPACE_SW);
    h.cupsNumColors = bits == 24 ? 3 : 1;
    h.NumCopies = 1;
    h.cupsInteger[CUPS_RASTER_PWG_TotalPageCount] = 2;
    if (!cupsRasterWriteHeader2(r, &h)) return 1;
    unsigned char *line = malloc(h.cupsBytesPerLine);
    for (unsigned y = 0; y < h.cupsHeight; y++) {
      memset(line, bits == 1 ? 0 : 255, h.cupsBytesPerLine);
      if (y < 20) {
        if (bits == 24) {
          for (unsigned x = 0; x < 10; x++) {
            memset(line + 3 * x, 0, 3);
            line[3 * x + p] = 255;
          }
        } else if (bits == 1) {
          line[0] = 0xf0;
        } else {
          memset(line, p ? 64 : 0, 10);
        }
      }
      if (cupsRasterWritePixels(r, line, h.cupsBytesPerLine) != h.cupsBytesPerLine) return 1;
    }
    free(line);
  }
  cupsRasterClose(r); close(fd); return 0;
}
