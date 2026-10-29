// Pure app-shelf helpers for Hide Other Apps (FR-43). Lists hold bare
// executable file names and match case-insensitively (spec 5.1); an app
// is never on both shelves, so every helper keeps the two lists disjoint.

/** Is `a` the same executable as `b`? Bare names are case-insensitive. */
function sameName(a: string, b: string): boolean {
  return a.toLowerCase() === b.toLowerCase();
}

/** Is `name` a bare executable file name like Zoom.exe (no directories,
 * no drive letters, exactly .exe)? Mirrors settings.rs is_exe_name. */
export function isExeName(name: string): boolean {
  const trimmed = name.trim();
  return (
    trimmed.length > 4 &&
    trimmed.toLowerCase().endsWith(".exe") &&
    !/[/\\:*?"<>|]/.test(trimmed)
  );
}

/** Append a typed app name. Returns null when it is not a bare executable
 * file name like Zoom.exe, or is already on the list. */
export function addApp(list: string[], name: string): string[] | null {
  const trimmed = name.trim();
  if (!isExeName(trimmed)) {
    return null;
  }
  if (list.some((entry) => sameName(entry, trimmed))) return null;
  return [...list, trimmed];
}

/** Drop `name`, matching case-insensitively. */
export function removeApp(list: string[], name: string): string[] {
  return list.filter((entry) => !sameName(entry, name));
}

/** Move `name` from `from` to `to`: the returned pair is the next settings'
 * Stays on Screen and Hidden lists, so one save covers both. When the
 * target already holds the app (any casing) its spelling wins and the
 * source copy is simply dropped. */
export function moveApp(
  from: string[],
  to: string[],
  name: string,
): { from: string[]; to: string[] } {
  const entry = from.find((candidate) => sameName(candidate, name)) ?? name;
  const stays = removeApp(from, entry);
  if (to.some((candidate) => sameName(candidate, entry))) {
    return { from: stays, to: [...to] };
  }
  return { from: stays, to: [...to, entry] };
}
