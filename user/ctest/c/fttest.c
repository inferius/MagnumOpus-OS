/*
 * Test portu FreeType (docs/cs/porty-c.md): staticky program nad
 * libfreetype.a. Nacte pismo (argument 1), vykresli glyf "A" ve 32 px
 * a overi rozmery bitmapy, inkoust a posun.
 *
 * Vystup: "ok <cast>" a na konci "fttest: hotovo" (protokol testu initu).
 */

#include <ft2build.h>
#include FT_FREETYPE_H
#include <stdio.h>

int main(int argc, char **argv)
{
    FT_Library lib;
    FT_Face face;
    FT_Int major, minor, patch;
    unsigned ink = 0;

    if (argc < 2) return 2;
    if (FT_Init_FreeType(&lib)) return 3;
    FT_Library_Version(lib, &major, &minor, &patch);
    printf("ok init %d.%d.%d\n", major, minor, patch);
    if (FT_New_Face(lib, argv[1], 0, &face)) return 4;
    if (face->num_glyphs <= 0 || !(face->face_flags & FT_FACE_FLAG_SCALABLE)) return 5;
    printf("ok face %s\n", face->family_name ? face->family_name : "?");
    if (FT_Set_Pixel_Sizes(face, 0, 32)) return 6;
    if (FT_Load_Char(face, 'A', FT_LOAD_RENDER)) return 7;
    {
        FT_Bitmap *b = &face->glyph->bitmap;
        unsigned x, y;
        if (b->pixel_mode != FT_PIXEL_MODE_GRAY || b->width < 10 || b->rows < 15 || b->rows > 40) return 8;
        for (y = 0; y < b->rows; y++)
            for (x = 0; x < b->width; x++)
                if (b->buffer[y * b->pitch + x] > 128) ink++;
        /* Glyf "A" ma vyplnenou zhruba desetinu az polovinu ctverce. */
        if (ink < b->width * b->rows / 10 || ink > b->width * b->rows / 2) return 9;
        if (face->glyph->advance.x <= 0) return 10;
        printf("ok glyf %ux%u inkoust %u\n", b->width, b->rows, ink);
    }
    FT_Done_Face(face);
    FT_Done_FreeType(lib);
    printf("fttest: hotovo\n");
    return 0;
}
