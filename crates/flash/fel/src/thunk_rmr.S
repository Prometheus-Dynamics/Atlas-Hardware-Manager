@ FEL thunk, AArch32 FEL to an AArch64 payload through an RMR warm reset
@ (mode a; clean-room, written from the ARMv8-A architecture and the
@ functional need). The design is in the crate root (src/lib.rs) and the
@ mode in src/layout.rs (ThunkIsa::Aarch32ToAarch64).
@
@ The host writes this whole blob at the layout's thunk_address, sets the
@ swap table (SWAP_ENTRIES triples of buf1, buf2, bytes; a zero byte count
@ ends it), the AArch64 entry word and the reset vector word (the SoC's
@ writable RVBAR backing store, from the layout), and runs fel_thunk_rmr
@ through FEL_RUN.
@
@ The RMR write is MCR p15, 0, r2, c12, c0, 2 (AArch32 RMR, the low 32 bits of
@ RMR_EL3; EL3 only). At EL2 or EL1 it is UNDEFINED. The exception level the
@ boot ROM's FEL runs at is UNCONFIRMED: docs/prs/D16.md, items 8 and 10.
@
@ Steps: swap every table buffer so the SPL's bytes sit at their load
@ address; write the AArch64 entry to the RVBAR backing store; write RMR
@ with AA64 = 1 and RR = 1 (the architecture's warm reset into AArch64 at
@ the reset vector). The core resets, so this code never returns: there is
@ no save area and no return into the boot ROM. The trailing loop only
@ waits for the reset.
@
@ The swap pass is the one of thunk.S. It uses r0-r3 and r12 only, and the
@ code never touches the stack.
@
@ Assembled with: clang --target=armv7a-none-eabi -c src/thunk_rmr.S (the
@ same tool as thunk.S). The Rust words in src/thunk.rs must equal the
@ assembled .text bytes; the test in src/thunk.rs checks that when clang is
@ installed.

    .syntax unified
    .arm
    .section .text,"ax",%progbits
    .global fel_thunk_rmr
fel_thunk_rmr:
    adr     r4, table
next_entry:
    ldr     r0, [r4], #4
    ldr     r1, [r4], #4
    ldr     r2, [r4], #4
    cmp     r2, #0
    beq     swaps_done
swap_word:
    ldr     r3, [r0]
    ldr     r12, [r1]
    str     r12, [r0], #4
    str     r3, [r1], #4
    subs    r2, r2, #4
    bne     swap_word
    b       next_entry
swaps_done:
    adr     r0, vector_word
    ldr     r0, [r0]
    adr     r1, entry_word
    ldr     r1, [r1]
    str     r1, [r0]
    dsb     sy
    mov     r2, #3
    mcr     p15, 0, r2, c12, c0, 2
    dsb     sy
    isb     sy
halt:
    wfi
    b       halt

    .balign 4
table:
    .space 8 * 12
entry_word:
    .word 0
vector_word:
    .word 0
