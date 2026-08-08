export function isManualCompactionCommand(value: string): boolean {
  return value.trim() === "/compact";
}
