import type { LogicalRect } from "@hachimi/contracts";

export interface RectLike {
  x: number;
  y: number;
  width: number;
  height: number;
}

export function exceedsDragThreshold(
  start: Pick<PointerEvent, "clientX" | "clientY">,
  current: Pick<PointerEvent, "clientX" | "clientY">,
  threshold = 5,
): boolean {
  return Math.hypot(current.clientX - start.clientX, current.clientY - start.clientY) > threshold;
}

export function collectInteractiveRects(
  elements: readonly (Element | undefined)[],
  viewportWidth: number,
  viewportHeight: number,
): LogicalRect[] {
  return elements
    .filter((element): element is Element => Boolean(element))
    .filter((element) => element.isConnected !== false)
    .map((element) => element.getBoundingClientRect())
    .filter((rect) => rect.width > 0 && rect.height > 0)
    .map((rect) => {
      const left = Math.max(0, rect.x);
      const top = Math.max(0, rect.y);
      const right = Math.min(viewportWidth, rect.x + rect.width);
      const bottom = Math.min(viewportHeight, rect.y + rect.height);
      return { x: left, y: top, width: right - left, height: bottom - top };
    })
    .filter((rect) => rect.width > 0 && rect.height > 0)
    .slice(0, 8);
}

export interface PetInteractiveElements {
  silhouette?: Element | undefined;
  actionBar?: Element | undefined;
  composer?: Element | undefined;
  menuContent?: Element | undefined;
  actionsVisible: boolean;
  menuOpen: boolean;
}

export function collectPetInteractiveRects(
  elements: PetInteractiveElements,
  viewportWidth: number,
  viewportHeight: number,
): LogicalRect[] {
  return collectInteractiveRects(
    [
      elements.silhouette,
      elements.actionsVisible ? elements.actionBar : undefined,
      elements.composer,
      elements.menuOpen ? elements.menuContent : undefined,
    ],
    viewportWidth,
    viewportHeight,
  );
}
