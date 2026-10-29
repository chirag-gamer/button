// Countdown overlay (FR-41, spec 7.1): the window opens with "3" already
// on screen (it is in countdown.html) and this page swaps the digit on
// each countdown tick. The core creates and closes the window
// (countdown.rs) and the page only listens: its capability grants events,
// no commands.
import "./countdown.css";
import "../shared/theme.css";
import { onCountdown } from "../shared/ipc";

const digit = document.getElementById("digit")!;

/** Swap the digit for the tick. The payload is already 3|2|1, so writing
 * the number is enough and a late or repeated event is harmless. */
function show(secondsLeft: number): void {
  digit.textContent = String(secondsLeft);
}

void onCountdown(show);
