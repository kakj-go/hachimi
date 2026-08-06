import type {
  RunEventEnvelope,
  TranscriptItemKind,
  UserInputRequestRecord,
} from "@hachimi/contracts";

/**
 * Process-local streaming projection.  Deltas are deliberately kept separate
 * from the transcript projection: an authoritative item.completed payload
 * always wins and removes the live buffer for that item.
 */
export type LiveItemDelta = {
  text: string;
  kind?: TranscriptItemKind;
};

export type LiveItemDeltas = Readonly<Record<string, LiveItemDelta>>;

export function reduceLiveItemDeltas(
  current: LiveItemDeltas,
  events: readonly RunEventEnvelope[],
): Record<string, LiveItemDelta> {
  const next: Record<string, LiveItemDelta> = { ...current };
  for (const event of events) {
    switch (event.payload.type) {
      case "item_started":
        next[event.payload.data.item.id] ??= {
          text: "",
          kind: event.payload.data.item.kind,
        };
        break;
      case "item_delta": {
        const item = next[event.payload.data.item_id];
        // A delta without a matching started event is stale (most commonly a
        // replay that arrived after the authoritative completion). Never let
        // it resurrect a completed transcript item.
        if (!item) break;
        // Keep at most 256 KiB in the WebView even if a provider streams an
        // unbounded response.  This is only a display cache, never a result.
        const delta = event.payload.data.delta;
        const text =
          delta.type === "text" || delta.type === "command_output" ? delta.data.text : "";
        item.text = `${item.text}${text}`.slice(-262_144);
        next[event.payload.data.item_id] = item;
        break;
      }
      case "item_completed":
        // The completed payload is authoritative, so a late delta can never
        // overwrite it after the event batch has been reduced in order.
        delete next[event.payload.data.item.id];
        break;
      default:
        break;
    }
  }
  return next;
}

export function liveItemText(item: LiveItemDelta | undefined): string | undefined {
  return item?.text;
}

/**
 * Pending UserInput records are immutable and identified by a stable broker ID.
 * Reuse an existing record while it remains pending so a projection refresh
 * cannot remount its card and discard an in-memory (possibly secret) answer.
 */
export function reconcilePendingUserInputs(
  current: readonly UserInputRequestRecord[],
  incoming: readonly UserInputRequestRecord[],
): UserInputRequestRecord[] {
  const existing = new Map(current.map((request) => [request.id, request]));
  return incoming.map((request) => existing.get(request.id) ?? request);
}
