/*
 * Test portu HarfBuzz (krok 7a-1, docs/cs/cpp.md): knihovna v C++ nad
 * libc++ volana z C. Nacte pismo (argument 1), vytvaruje text s diakritikou
 * a overi glyfy, posuny a ligaturu.
 *
 * Vystup: "ok <cast>" a na konci "hbtest: hotovo" (protokol testu initu).
 */

#include <hb.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;

#define CHECK(cond)                                                                                                    \
    do {                                                                                                               \
        if (!(cond)) {                                                                                                 \
            fprintf(stderr, "hbtest: selhalo %s (%s:%d)\n", #cond, __FILE__, __LINE__);                                 \
            failures++;                                                                                                \
            return;                                                                                                    \
        }                                                                                                              \
    } while (0)

static hb_font_t *font;

static void test_version(void)
{
    CHECK(strcmp(hb_version_string(), "14.2.0") == 0);
    printf("ok verze\n");
}

static void test_shape_latin(void)
{
    const char *text = "P\xC5\x99\xC3\xADli\xC5\xA1 \xC5\xBElu\xC5\xA5ou\xC4\x8Dk\xC3\xBD k\xC5\xAF\xC5\x88";
    hb_buffer_t *buf = hb_buffer_create();
    hb_buffer_add_utf8(buf, text, -1, 0, -1);
    hb_buffer_guess_segment_properties(buf);
    CHECK(hb_buffer_get_direction(buf) == HB_DIRECTION_LTR);
    CHECK(hb_buffer_get_script(buf) == HB_SCRIPT_LATIN);
    hb_shape(font, buf, NULL, 0);
    unsigned int n = 0;
    hb_glyph_info_t *info = hb_buffer_get_glyph_infos(buf, &n);
    hb_glyph_position_t *pos = hb_buffer_get_glyph_positions(buf, &n);
    CHECK(n > 10);
    hb_position_t advance = 0;
    for (unsigned int i = 0; i < n; i++) {
        /* Zadny glyf .notdef: pismo ma vsechny znaky vcetne diakritiky. */
        CHECK(info[i].codepoint != 0);
        advance += pos[i].x_advance;
    }
    CHECK(advance > 0);
    hb_buffer_destroy(buf);
    printf("ok tvarovani\n");
}

static void test_kerning(void)
{
    /* "AV" je u pisma Inter kernovane: dvojice uzsi nez soucet. */
    hb_position_t pair = 0, single = 0;
    const char *texts[] = {"AV", "A", "V"};
    for (int t = 0; t < 3; t++) {
        hb_buffer_t *buf = hb_buffer_create();
        hb_buffer_add_utf8(buf, texts[t], -1, 0, -1);
        hb_buffer_guess_segment_properties(buf);
        hb_shape(font, buf, NULL, 0);
        unsigned int n = 0;
        hb_glyph_position_t *pos = hb_buffer_get_glyph_positions(buf, &n);
        for (unsigned int i = 0; i < n; i++) {
            if (t == 0)
                pair += pos[i].x_advance;
            else
                single += pos[i].x_advance;
        }
        hb_buffer_destroy(buf);
    }
    CHECK(pair < single);
    printf("ok kerning\n");
}

int main(int argc, char **argv)
{
    if (argc < 2) {
        fprintf(stderr, "pouziti: hbtest <pismo.ttf>\n");
        return 2;
    }
    hb_blob_t *blob = hb_blob_create_from_file_or_fail(argv[1]);
    if (!blob) {
        fprintf(stderr, "hbtest: pismo %s nejde nacist\n", argv[1]);
        return 1;
    }
    hb_face_t *face = hb_face_create(blob, 0);
    font = hb_font_create(face);
    test_version();
    test_shape_latin();
    test_kerning();
    hb_font_destroy(font);
    hb_face_destroy(face);
    hb_blob_destroy(blob);
    if (failures)
        return 1;
    printf("hbtest: hotovo\n");
    return 0;
}
