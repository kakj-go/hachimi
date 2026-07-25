export interface SerializedAutosaveOptions<T> {
  initial: T;
  delay?: number;
  save: (value: T) => Promise<T>;
  onConfirmed: (value: T) => void;
  onRollback: (value: T, error: unknown) => void;
  onStatusChange?: (status: AutosaveStatus) => void;
}

export type AutosaveStatus = "idle" | "pending" | "saving" | "saved" | "error";

export interface SerializedAutosave<T> {
  schedule: (value: T, immediate?: boolean) => void;
  flush: () => Promise<void>;
  accept: (value: T) => void;
  dispose: () => void;
}

export function createSerializedAutosave<T>(
  options: SerializedAutosaveOptions<T>,
): SerializedAutosave<T> {
  let confirmed = options.initial;
  let queued: T | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let running: Promise<void> | undefined;
  let disposed = false;

  const clearTimer = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };

  const drain = async () => {
    if (running) return running;
    running = (async () => {
      clearTimer();
      while (!disposed && queued !== undefined) {
        const next = queued;
        queued = undefined;
        options.onStatusChange?.("saving");
        try {
          confirmed = await options.save(next);
          options.onConfirmed(confirmed);
          options.onStatusChange?.("saved");
        } catch (error) {
          queued = undefined;
          options.onRollback(confirmed, error);
          options.onStatusChange?.("error");
          break;
        }
      }
    })().finally(() => {
      running = undefined;
    });
    return running;
  };

  return {
    schedule(value, immediate = false) {
      if (disposed) return;
      queued = value;
      options.onStatusChange?.("pending");
      clearTimer();
      if (immediate) {
        void drain();
      } else {
        timer = setTimeout(() => void drain(), options.delay ?? 250);
      }
    },
    async flush() {
      clearTimer();
      await drain();
      if (running) await running;
    },
    accept(value) {
      if (!running && queued === undefined) confirmed = value;
    },
    dispose() {
      disposed = true;
      clearTimer();
      queued = undefined;
    },
  };
}
