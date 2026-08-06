import { type Accessor, createEffect, createSignal, onCleanup, untrack } from "solid-js";

const BOTTOM_THRESHOLD_PX = 32;

export function createSessionScrollController(options: {
  viewport: Accessor<HTMLElement | undefined>;
  content: Accessor<HTMLElement | undefined>;
  revision: Accessor<string>;
  sessionKey: Accessor<string | undefined>;
}) {
  const [atBottom, setAtBottom] = createSignal(true);
  let previousSessionKey: string | undefined;

  const measure = (viewport: HTMLElement) => {
    const distance = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;
    setAtBottom(distance < BOTTOM_THRESHOLD_PX);
  };

  const followLatest = (behavior: ScrollBehavior = "auto") => {
    const viewport = options.viewport();
    if (!viewport) return;
    if (typeof viewport.scrollTo === "function") {
      viewport.scrollTo({ top: viewport.scrollHeight, behavior });
    } else {
      viewport.scrollTop = viewport.scrollHeight;
    }
    if (behavior === "auto") measure(viewport);
  };

  createEffect(() => {
    const viewport = options.viewport();
    const content = options.content();
    if (!viewport) return;

    const onScroll = () => measure(viewport);
    viewport.addEventListener("scroll", onScroll, { passive: true });
    measure(viewport);

    const observer =
      typeof ResizeObserver === "undefined"
        ? undefined
        : new ResizeObserver(() => {
            const shouldFollow = untrack(atBottom);
            requestFrame(() => {
              if (shouldFollow) followLatest();
              else measure(viewport);
            });
          });
    observer?.observe(viewport);
    if (content) observer?.observe(content);

    onCleanup(() => {
      viewport.removeEventListener("scroll", onScroll);
      observer?.disconnect();
    });
  });

  createEffect(() => {
    options.revision();
    const sessionKey = options.sessionKey();
    const sessionChanged = sessionKey !== previousSessionKey;
    previousSessionKey = sessionKey;
    const shouldFollow = sessionChanged || untrack(atBottom);
    requestFrame(() => {
      const viewport = options.viewport();
      if (!viewport) return;
      if (shouldFollow) followLatest();
      else measure(viewport);
    });
  });

  return {
    atBottom,
    scrollToBottom: () => followLatest("smooth"),
  };
}

function requestFrame(callback: () => void) {
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(callback);
  } else {
    queueMicrotask(callback);
  }
}
