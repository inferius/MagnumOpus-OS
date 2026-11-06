/*
 * Linker skript kernelu (x86_64).
 *
 * Kernel lezi v poslednich 2 GiB adresniho prostoru (code-model=kernel).
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

    .text : ALIGN(4K)
    {
        *(.text .text.*)
    } :text

    .rodata : ALIGN(4K)
    {
        *(.rodata .rodata.*)
    } :rodata

    .eh_frame_hdr : { *(.eh_frame_hdr) } :rodata
    .eh_frame : { KEEP(*(.eh_frame)) } :rodata

    .data : ALIGN(4K)
    {
        *(.data .data.*)
        *(.got .got.*)
    } :data

    .bss : ALIGN(4K)
    {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    /DISCARD/ :
    {
        *(.comment)
        *(.note .note.*)
    }
}
