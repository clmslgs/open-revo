# Xingyao 14 P916F-PTL

This contract is scoped to the inspected MECHREVO `XINGYAO Series-P916F-PTL`,
Intel Core Ultra X7 358H, BIOS 1.00 dated 2026-03-14. STX and LNL protocols
have not been validated on this machine.

Inspected DSDT SHA-256:
`a6e21629835d72c9c8c7fbdfec31ae7ee9dcb99425a6044f0747590d2730392d`.

The exact PTL contract is selected before the older `P916F*` contract. Modes and key switches use
`root\wmi:OemWMIMethod.OemWMIfun`, with a 64-byte input and 256-byte output.
Read responses require byte 0 to be zero. Performance-mode writes return the
EC command echo in byte 0: `0x94` / `0x95` / `0x96` (148 / 149 / 150).
A successful WMI transport call alone is insufficient.

| Operation | Request | Response | Evidence |
| --- | --- | --- | --- |
| Interface version | `01 01` | little-endian DWORD at byte 1 | Local DSDT `GVER`, live value `0x00020004` |
| Fan selector 0 | `02 01 00` | little-endian WORD at byte 1 | Local DSDT `GFNS`, live query |
| Fan selector 1 | `02 01 01` | little-endian WORD at byte 1 | Local DSDT `GFNS`, live query |
| Current performance mode | `03 09` | byte 1: 1 / 2 / 3 | Local DSDT `GPFM`, live query |
| Performance mode | `03 0A mode` | byte 0: `94` / `95` / `96` (hex) | Local DSDT `SPFM`: office / balance / turbo; live writes and readback |
| Fn lock | `03 04 enabled` | status byte | Local DSDT `SFLS` |
| Windows-key lock | `03 02 enabled` | status byte | Local DSDT `SWLS` |

Fan selectors require separate calls. The second fan is not a second WORD in
the first response. Zero is a valid stopped-fan reading and clears cached RPM.
On this machine both WMI selectors remained zero even while PM2 tachometers
measured fans spinning near 4000/4500 RPM. They are retained only as diagnostic
actions. Production `read_fan_rpm` explicitly selects the PM2 channel; both the
display and manual controller use its high/low/high coherent tachometer reads.

## Performance-mode acknowledgement regression

Treating all WMI responses as status-zero replies made successful mode writes
fail validation. The EC had already entered Office, but the command returned an
error before updating the application state and saved preference. When the UI's
five-second optimistic display expired, it showed the old Turbo state again.

The `set_power_mode` action now declares its channel and expected acknowledgement
separately for each mode. Read actions retain the channel's status-zero rule;
actual error statuses and another mode's echo remain rejected. Before committing
the cached mode, the driver confirms `get_power_mode` readback within 400 ms.
Readback failures cannot fall back to a cached Turbo value. The frontend clears
its optimistic display immediately when a command really fails.

Live regression checks passed Balance, Turbo, and Office switching and ten
consecutive one-second Office readbacks. The probe restores its original mode
on exit; the inspected machine started and ended in Office.

The inspected WMAA dispatcher does not expose charging limits or manual fans.
These controls instead use the declared `pmc_ec` channel, protocol
`xingyao_ptl_v1`, implemented in the shared HAL. Native access checks the actual
board, manufacturer and BIOS 1.00 independently of the JSON declaration.
No Tongfang or Baolongda EC register map is used.

The existing `inpoutx64` port driver is required, with SHA-256
`f8965fdce668692c3785afa3559159f9a18287bc0d53abb21902895a8ecf221b`.
The backend opens its existing device; it does not install a driver or disable
Windows protections. The signed PawnIO CPU Package temperature reader is also
required before entering manual fan control.

## PM2 protocol and firmware evidence

The local DSDT maps PM2 data/status-command ports to `0x68`/`0x6c`.
Transactions serialize through the same named mutex as
[FanControl.MechrevoPtl](https://github.com/Kurisu934/FanControl.MechrevoPtl).
Every handshake has a one-second timeout, status `0xff` is rejected, and only
one persistent stale output byte may be consumed between transactions.
Partially sent commands are never automatically repeated.

Read-only extraction of the EC image from the host firmware window produced
SHA-256 `5989b63c442e909790e8da0314cc84564296a23f6c81bfc1eb92a4137ed91857`,
with header `ITE51300-SB-V.00120230800` and version
`ITE Tech. Inc. $INTEL PTL$MECHREVO$A0$VER:01.00.00$`.
This is the host-stored image, not a hash of independently read active EC flash.
The image and full machine dumps are kept out of distributed resources.

| Command | Parameter | Effect / local image evidence |
| --- | --- | --- |
| `D5` | `10` | Restore both fans to automatic control |
| `D6` | PWM byte | Shared PWM, `round(percent * 255 / 100)`; handler `C8CE` |
| `D7` / `D8` | RPM / 100 | Independent target RPM; handlers `C8FE` / `C922` |
| `D5` | `18/19`, `16/17` | Fan 0 / 1 low/high RPM bytes |
| `F1` | `10` / `11` | Disable / enable charging thresholds; dispatcher `C682` |
| `F1` | `12/13/14` | Read enable / charge boundary / discharge boundary |
| `F2` / `F3` | 0–100 | Set charge / discharge boundary, echo the written value; `C750` / `C790` |

The charging routine at `7A88` reads the two boundaries from EC-internal RAM
`0x800D08` and `0x800D09`. These are internal EC addresses, not host ACPI indices.
The second boundary may request **active discharge**. A charging cap sets it to
100%, sets the first boundary to 60% or 80%, then enables the policy. Selecting
100% disables the policy. Every update verifies both echoes and all three
readback values, and attempts to restore the previous state if an update fails.
These commands were verified from the PTL image and live behavior, independently
of superficially similar STX commands.

## Controls, validation and limits

The overview's “手动转速与风扇曲线” button opens a dedicated control panel without
requiring a fourth performance mode. Manual targets are independent, in
100-RPM steps, between 2000 and 7500 RPM. Curves use CPU Package temperature
and apply shared PWM to both fans, with a 25% floor. The 7500-RPM command ceiling
is a target limit, not a guarantee of the motors' attainable speed.

Curve percentages are **PWM duty commands**, not percentages of maximum RPM.
25% encodes byte 64/255 and does not request 2200 RPM. The old generic editor
footer incorrectly attached that RPM label to the PTL PWM floor; it now labels
PWM explicitly. The panel shows the CPU Package temperature used by the curve
and its requested PWM alongside live fan tachometer readings.
For a fixed RPM target, use manual RPM control instead. Active curve settings
are saved into the active profile file, which is the source used at startup.

The software curve runs at 1 Hz even while the main window is hidden. A missing
temperature, temperature at or above 90°C, a communication error, a scheduling
gap longer than five seconds, suspend, shutdown, or normal application exit
cancels manual control and restores automatic control. Failed restoration is
reported as `recovering` and retried without reapplying manual targets.
Curves therefore require OpenRevo to remain running. There is no independent
watchdog for a process crash or forced termination; restoration cannot be
guaranteed in that case. Actual sleep/wake cycling and sustained thermal loads
have not been tested in this session.

Live checks on BIOS 1.00:

- Requested 4000/4500 RPM, shared-core backend measured 3913/4445 RPM.
- After switching production tachometer reads to PM2, the HAL display returned
  3963/4436 RPM, matching the controller's own measurement.
- A constant 50% curve measured 5337/4922 RPM; PWM encoding was 128/255.
- Low-speed 2000/2000 targets measured 1962/1906 RPM; the minimum 25% PWM
  curve kept both motors running at 2953/2594 RPM.
- 60% / 80% / 100% charging policies all passed hardware readback.
- With battery charge above the 80% boundary, charging changed from 7730 mW to
  0 mW; `Charging=false`, `Discharging=false`, external power remained online.
- Test cleanup restored automatic fans and the original disabled charging cap.

Protocol tests cover partial-write non-retry, charge-update rollback,
discharge-boundary ordering, torn tachometer samples, curve validation and
high-temperature rejection. Driver errors reach the command/UI before saving
configuration. A WMI mailbox alone never selects the Baolongda controller.

Intel CPU Package temperature uses the separately documented PawnIO sensor.
Read-only checks can be repeated with:

```powershell
cargo run --offline -p openrevo-core --example ptl_contract_probe
```

The separate `ptl_control_probe -- --verify-controls` example performs bounded
reversible real-hardware tests; its default invocation only reads state.

Power-mode regression can be checked with
`cargo run --offline -p openrevo-core --example ptl_power_mode_probe -- --verify-switches`.
Without `--verify-switches`, that probe only reads the active hardware mode.

`cargo run --offline -p openrevo-core --example ptl_control_probe -- --sample-fans`
samples CPU Package temperature and both tachometers eight times without
changing fan policy or charging settings. This can run alongside an active
software curve. A repeated sample while the panel reported PWM 25% measured
3071–3084 / 2746–2753 RPM on this machine.
