# Raze: the USB link stops sending

What the bench board showed on 2026-10-17 to 2026-10-19, so it's recognisable
if a robot does it. The likeliest cause is the bench itself: the board runs on
a PC USB port at about 4.2 V, and most of that host's enumerations of it
logged "device descriptor read/64, error -71". As of 2026-10-19 a host-side
usbmon and xhci trace is set to catch the next stall from both sides.
`board-usb-gadget-watch` is the safety net either way.

## The symptom

The board's USB network (ECM, `usb0` in the `usbbr0` bridge) keeps receiving
but stops sending. The host gets no DHCP lease or stops seeing replies.
dnsmasq logs "Error sending DHCP packet ... Resource temporarily unavailable".
Carrier stays up, the UDC says "configured", and neither the kernel nor the
host logs anything. It hit at boot (after a -71 enumeration) and mid-run, from
28 minutes to about 14 hours of uptime. Only rebinding the gadget, so the host
re-enumerates it, brought it back.

## What the captures show

`usb-gadget-watch` saves the controller's state before each rebind, in
`/data/board/usb-gadget-watch/<n>-<boot>/`. All three captures look the same:

- ep1in (ECM bulk IN, TxFIFO 4) is enabled and armed. `DIEPCTL=0x81098200`,
  and `DIEPTSIZ` has PktCnt 1 or 2 with XferSize 0. Ten requests are queued.
- The head request is already in the FIFO. The used words from regdump's
  `DTXFSTS(1)` match its length: 913 B with 0x11b free, 687 B with 0x154,
  42 B with 0x1f5. So DMA delivered the data and the FIFO bookkeeping is right.
- `DCTL=0` (no global NAK), and `DSTS` shows the bus isn't suspended.
- The controller's interrupt count doesn't change over one second.
- On the host, `rx_packets` stops at exactly the board's `tx_packets`, and
  `rx_errors` stays 0. usbnet counts URB errors (EPROTO, EPIPE, ETIME) there,
  so every frame that was sent arrived, and the host's bulk-IN URBs didn't fail.
  They just stopped completing.

So the packet waits in the FIFO and no IN token takes it. Either the host
controller stops scheduling the endpoint, or dwc2 NAKs with data loaded.
Telling them apart needs a bus-level view: host xhci debugfs or usbmon at the
stall, a USB analyser, or another port, cable or host.

## Ruled out

- **The ZLP case.** A 30-minute flood of about 3.6 M frames of exactly 512 or
  1024 bytes didn't stall it, and the stuck heads aren't multiples of 512.
- **TX FIFO size.** dwc2 caps each FIFO at its hardware depth, which is 512
  words for FIFO 4. A larger `g-tx-fifo-size` is refused with "Invalid
  parameter g_tx_fifo_size[4]".
- **Unread ACM consoles.** It stalled with the host draining both
  `/dev/ttyACM*` ports, while ep3in and ep5in were empty and idle.
- **Host autosuspend.** `power/control` was `on`.
- **NCM instead of ECM.** NCM soaked 4 hours clean, but it costs about 12 times
  the round-trip time. A Windows Driver Station uses the RNDIS configuration
  anyway.

## The safety net

`board-usb-gadget-watch` probes every 5 s. After three probes in a row in which
a bridge member sent nothing, it saves the evidence and rebinds the UDC, so a
stall costs about 15 s of link. It rebinds at most once every 5 minutes and 3
times a boot, and never while the host has the bus suspended.
`usb-gadget.rebound` events in `events.jsonl` count the stalls. If they show up
on a robot, look at the evidence dirs first. If the host is Windows, whether it
recovers by itself is untested; the rebind recovers it either way.
