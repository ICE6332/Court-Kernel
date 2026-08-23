OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)

SECTIONS
{
    . = 0x00100000;

    .text : {
        *(.text .text.*)
    }

    .rodata : {
        *(.rodata .rodata.* .srodata .srodata.*)
    }

    .data : {
        *(.data .data.* .sdata .sdata.*)
    }

    .bss : {
        *(.bss .bss.* .sbss .sbss.*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note*)
        *(.comment*)
        *(.got*)
    }
}
