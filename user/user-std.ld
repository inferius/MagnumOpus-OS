/*
 * Linker skript aplikaci se std (cil *-unknown-magnumopus, x86_64 i aarch64).
 *
 * Stejne rozlozeni jako user/user.ld (staticky ET_EXEC od 4 MiB, skupiny
 * sekci R-X, R--, RW- na vlastnich strankach zarovnanych na 64 KiB) a navic
 * TLS: .tdata i .tbss jsou v jedne vystupni sekci, takze cela sablona TLS
 * (vcetne nul z .tbss) je v souboru a runtime (libs/osrt, tls.rs) ji jen
 * zkopiruje do bloku vlakna. Segment PT_TLS urcuje posuny promennych
 * (p_memsz, p_align); symboly __tls_* je predavaji runtime.
 */

ENTRY(_start)

PHDRS
{
    text   PT_LOAD FLAGS(5);   /* R-X */
    rodata PT_LOAD FLAGS(4);   /* R-- */
    data   PT_LOAD FLAGS(6);   /* RW- */
    tls    PT_TLS;
}

SECTIONS
{
    . = 0x400000;

    .text : ALIGN(64K) { *(.text .text.*) } :text
    .rodata : ALIGN(64K) { *(.rodata .rodata.*) } :rodata
    .eh_frame_hdr : { *(.eh_frame_hdr) } :rodata
    .eh_frame : { KEEP(*(.eh_frame)) } :rodata
    .gcc_except_table : { *(.gcc_except_table .gcc_except_table.*) } :rodata
    .data : ALIGN(64K) { *(.data .data.*) *(.got .got.*) } :data
    .init_array : { KEEP(*(SORT_BY_INIT_PRIORITY(.init_array.*) .init_array)) } :data
    .tdata : {
        __tls_template_start = .;
        *(.tdata .tdata.*)
        *(.tbss .tbss.*)
        *(.tcommon)
        /* Vynuti PROGBITS: bez .tdata by sekce byla NOBITS a lld by jeji
         * adresy prekryl s nasledujicimi sekcemi (sablona by nebyla nuly). */
        BYTE(0)
    } :data :tls
    __tls_template_size = SIZEOF(.tdata);
    __tls_align = ALIGNOF(.tdata);
    .bss : ALIGN(4K) { *(.bss .bss.*) *(COMMON) } :data

    /DISCARD/ : { *(.comment) *(.note .note.*) }
}
