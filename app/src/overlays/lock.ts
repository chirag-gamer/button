// Lock overlay (FR-34, spec 7.10): the mouse stays usable, so the Unlock
// button is the no-keyboard way out; the hook itself answers to Esc.
import { unlockKeyboard } from "../shared/ipc";

const button = document.getElementById("unlock") as HTMLButtonElement;
button.addEventListener("click", () => {
  void unlockKeyboard().catch(console.error);
});
