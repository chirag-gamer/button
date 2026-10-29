// Pure shelf helpers for the Switches section (FR-44). The three dropdown
// shelves (shown / "Show all" / hidden) always hold every switch exactly
// once; these functions keep that invariant without touching the DOM.
import { SwitchId } from "../shared/ipc";

/** The three dropdown shelves (spec 5.1 `dropdown`). */
export interface Shelves {
  shown: SwitchId[];
  more: SwitchId[];
  hidden: SwitchId[];
}

export type ShelfName = keyof Shelves;

/** A copy of every shelf, so callers never see a shared array. */
function clone(shelves: Shelves): Shelves {
  return {
    shown: [...shelves.shown],
    more: [...shelves.more],
    hidden: [...shelves.hidden],
  };
}

/** Move `id` onto `target`, removing it from the other two shelves and
 * appending it at the end of the target shelf. Moving an id onto the shelf
 * it already sits on changes nothing: reordering is the arrows' job.
 * Returns a new object, never mutating the input. */
export function moveBetween(
  shelves: Shelves,
  id: SwitchId,
  target: ShelfName,
): Shelves {
  if (shelves[target].includes(id)) return clone(shelves);
  const without = {
    shown: shelves.shown.filter((entry) => entry !== id),
    more: shelves.more.filter((entry) => entry !== id),
    hidden: shelves.hidden.filter((entry) => entry !== id),
  };
  return { ...without, [target]: [...without[target], id] };
}

/** Swap `list[index]` with its neighbour `delta` places away (the up and
 * down buttons pass -1 and +1). Clamped: an end row is a no-op. */
export function reorder<T>(list: T[], index: number, delta: number): T[] {
  const to = index + delta;
  if (index < 0 || index >= list.length || to < 0 || to >= list.length) {
    return [...list];
  }
  const next = [...list];
  const [moved] = next.splice(index, 1);
  next.splice(to, 0, moved);
  return next;
}
