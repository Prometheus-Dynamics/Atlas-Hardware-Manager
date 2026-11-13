@ FEL thunk for Atlas (clean-room; written from the functional need, not
@ from any vendor or community source). Its design is in the crate root (src/lib.rs).
@
@ The host writes this whole blob at the layout's thunk_address, then sets
@ the table (SWAP_ENTRIES triples of buf1, buf2, bytes; a zero byte count
@ ends it), the SPL entry word, and runs fel_thunk through FEL_RUN.
@
@ Steps: save the FEL registers and return address in save_area, swap every
@ table buffer, call the SPL entry, swap the buffers back, restore the
@ registers and return into the boot ROM's FEL loop.
@
@ Only r0-r3 and r12 are used as scratch between the save and the restore,
@ and the code never touches the stack while the stack buffers are swapped.
@ The swap is its own inverse, so both passes run the same loop.
@
@ Assembled with: clang --target=armv7a-none-eabi -c src/thunk.S
@ The Rust words in src/thunk.rs must equal the assembled .text bytes; the
@ test in src/thunk.rs checks that when clang is installed.

    .syntax unified
    .arm
    .section .text,"ax",%progbits
    .global fel_thunk
fel_thunk:
    adr     r0, save_area
    stmia   r0!, {r4-r11}
    str     sp, [r0], #4
    str     lr, [r0], #4
    mov     r1, #0
    adr     r2, flag
    str     r1, [r2]
swap_pass:
    adr     r4, table
next_entry:
    ldr     r0, [r4], #4
    ldr     r1, [r4], #4
    ldr     r2, [r4], #4
    cmp     r2, #0
    beq     pass_end
swap_word:
    ldr     r3, [r0]
    ldr     r12, [r1]
    str     r12, [r0], #4
    str     r3, [r1], #4
    subs    r2, r2, #4
    bne     swap_word
    b       next_entry
pass_end:
    adr     r2, flag
    ldr     r1, [r2]
    cmp     r1, #0
    bne     restore
    mov     r1, #1
    str     r1, [r2]
    adr     r0, entry_word
    ldr     r0, [r0]
    blx     r0
    b       swap_pass
restore:
    adr     r0, save_area
    ldmia   r0!, {r4-r11}
    ldr     sp, [r0], #4
    ldr     lr, [r0], #4
    bx      lr

    .balign 4
table:
    .space 8 * 12
entry_word:
    .word 0
flag:
    .word 0
save_area:
    .space 10 * 4
