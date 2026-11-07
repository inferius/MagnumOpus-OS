/*
 * Linker skript programu v userspace (x86_64 i aarch64).
 *
 * Programy jsou staticke (ET_EXEC) od adresy 4 MiB; pod ni zustava pamet
 * nenamapovana, aby dereference nuloveho ukazatele skoncila chybou.
 * Kazda skupina sekci s jinymi pravy zacina na nove strance (R-X, R--, RW-),
 * W^X vynucuje jadro pri mapovani. Zarovnani je na nejvetsi moznou stranku
 * procesu (abi::vm::MAX_PAGE_SIZE = 64 KiB), aby sel stejny program nahrat
 * s jakoukoli velikosti stranky; v souboru to nic nestoji (jen mezery
 * v adresach).
 */

ENTRY(_start)

PHDRS
{
    text   PT_LOAD FLAGS(5);   /* R-X */
    rodata PT_LOAD FLAGS(4);   /* R-- */
    data   PT_LOAD FLAGS(6);   /* RW- */
}

SECTIONS
{
    . = 0x400000;

    .text : ALIGN(64K) { *(.text .text.*) } :text
    .rodata : ALIGN(64K) { *(.rodata .rodata.*) } :rodata
    .eh_frame_hdr : { *(.eh_frame_hdr) } :rodata
    .eh_frame : { KEEP(*(.eh_frame)) } :rodata
    .data : ALIGN(64K) { *(.data .data.*) *(.got .got.*) } :data
    .bss : ALIGN(4K) { *(.bss .bss.*) *(COMMON) } :data

    /DISCARD/ : { *(.comment) *(.note .note.*) }
}
