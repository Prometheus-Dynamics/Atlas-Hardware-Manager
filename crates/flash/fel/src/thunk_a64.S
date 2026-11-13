// FEL thunk, native AArch64 (mode b; clean-room, written from the ARMv8-A
// architecture and the functional need, not from any vendor or community
// source). The contract is the same as thunk.S (the ARMv7 thunk): the
// design is in the crate root (src/lib.rs) and the mode in src/layout.rs.
//
// The host writes this whole blob at the layout's thunk_address, sets the
// swap table (SWAP_ENTRIES triples of buf1, buf2, bytes; a zero byte count
// ends it) and the SPL entry word, then runs fel_thunk_a64 through FEL_RUN.
//
// Steps: save x19-x30 and sp in save_area, swap every table buffer, call the
// SPL entry (BLR), swap the buffers back, restore the registers and return
// into the boot ROM's FEL loop (RET through the restored x30).
//
// Only x0-x5 are used as scratch between the save and the restore, and the
// code never touches the stack: sp is saved and restored unchanged. Every
// table word is 32-bit (the SRAM is below 4 GiB), so the loads are w-forms
// that zero-extend into the x registers.
//
// Assembled with: aarch64-linux-gnu-as -o thunk_a64.o src/thunk_a64.S, then
// aarch64-linux-gnu-objcopy -O binary -j .text. The Rust words in
// src/thunk.rs must equal the assembled .text bytes; the test in src/thunk.rs
// checks that when the assembler is installed.

    .text
    .balign 4
    .global fel_thunk_a64
fel_thunk_a64:
    adr     x0, save_area
    stp     x19, x20, [x0], #16
    stp     x21, x22, [x0], #16
    stp     x23, x24, [x0], #16
    stp     x25, x26, [x0], #16
    stp     x27, x28, [x0], #16
    stp     x29, x30, [x0], #16
    mov     x1, sp
    str     x1, [x0]
    adr     x2, flag
    str     wzr, [x2]
swap_pass:
    adr     x4, table
next_entry:
    ldr     w0, [x4], #4
    ldr     w1, [x4], #4
    ldr     w2, [x4], #4
    cbz     w2, pass_end
swap_word:
    ldr     w3, [x0]
    ldr     w5, [x1]
    str     w5, [x0], #4
    str     w3, [x1], #4
    subs    w2, w2, #4
    b.ne    swap_word
    b       next_entry
pass_end:
    adr     x2, flag
    ldr     w1, [x2]
    cbnz    w1, restore
    mov     w1, #1
    str     w1, [x2]
    adr     x0, entry_word
    ldr     w0, [x0]
    blr     x0
    b       swap_pass
restore:
    adr     x0, save_area
    ldp     x19, x20, [x0], #16
    ldp     x21, x22, [x0], #16
    ldp     x23, x24, [x0], #16
    ldp     x25, x26, [x0], #16
    ldp     x27, x28, [x0], #16
    ldp     x29, x30, [x0], #16
    ldr     x1, [x0]
    mov     sp, x1
    ret

    .balign 8
table:
    .space 8 * 12
entry_word:
    .word 0
flag:
    .word 0
    .balign 8
save_area:
    .space 13 * 8
