/*
 * Linker skript kernelu (spolecny pro x86_64 i aarch64).
 *
 * Kernel lezi v poslednich 2 GiB adresniho prostoru (x86_64:
 * code-model=kernel; aarch64: vychozi model "small", adresy relativni k PC).
 * KERNEL_BASE musi odpovidat boot_info::KERNEL_BASE. Kazda skupina sekci
 * s jinymi pravy zacina na nove 4 KiB strance, aby je loader mohl
 * namapovat s presnymi pravy (R-X, R--, RW-).
 */

ENTRY(_start)

KERNEL_BASE = 0xFFFFFFFF80000000;

PHDRS
{
    text   PT_LOAD FLAGS(5);   /* R-X */
    rodata PT_LOAD FLAGS(4);   /* R-- */
    data   PT_LOAD FLAGS(6);   /* RW- */
}

SECTIONS
{
    . = KERNEL_BASE;

    /* Symboly __*_start / __*_end pouziva mm::map_kernel_image. */

    .text : ALIGN(4K)
    {
        __text_start = .;
        *(.text .text.*)
        __text_end = .;
    } :text

    .rodata : ALIGN(4K)
    {
        __rodata_start = .;
        *(.rodata .rodata.*)
    } :rodata

    .eh_frame_hdr : { *(.eh_frame_hdr) } :rodata
    .eh_frame : { KEEP(*(.eh_frame)) } :rodata
    __rodata_end = .;

    .data : ALIGN(4K)
    {
        __data_start = .;
        *(.data .data.*)
        *(.got .got.*)
    } :data

    .bss : ALIGN(4K)
    {
        *(.bss .bss.*)
        *(COMMON)
        __data_end = .;
    } :data

    /DISCARD/ :
    {
        *(.comment)
        *(.note .note.*)
    }
}
