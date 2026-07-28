export interface SequencedAgentEvent {
  sequence: number;
}

export interface EventWatermarkUpdate<T extends SequencedAgentEvent> {
  events: T[];
  nextSequence: number;
}

/**
 * Drops replayed events and advances the client watermark monotonically.
 * Session sequence values may have gaps because Transcript items share the
 * sequence allocator, so a numeric gap is not itself evidence of event loss.
 */
export function reduceAgentEventWatermark<T extends SequencedAgentEvent>(
  currentSequence: number,
  incoming: readonly T[],
): EventWatermarkUpdate<T> {
  const unique = new Map<number, T>();
  for (const event of incoming) {
    if (event.sequence > currentSequence && !unique.has(event.sequence)) {
      unique.set(event.sequence, event);
    }
  }
  const events = [...unique.values()].sort((left, right) => left.sequence - right.sequence);
  return {
    events,
    nextSequence: events.at(-1)?.sequence ?? currentSequence,
  };
}
