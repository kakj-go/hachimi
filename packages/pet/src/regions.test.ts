import { describe, expect, it } from "vitest";
import {
  collectInteractiveRects,
  collectPetInteractiveRects,
  exceedsDragThreshold,
} from "./regions";

function elementAt(x: number, y: number, width: number, height: number): Element {
  return {
    isConnected: true,
    getBoundingClientRect: () => ({ x, y, width, height }),
  } as unknown as Element;
}

describe("collectInteractiveRects", () => {
  it("distinguishes a click from a native-window drag", () => {
    const start = { clientX: 10, clientY: 10 };
    expect(exceedsDragThreshold(start, { clientX: 13, clientY: 14 })).toBe(false);
    expect(exceedsDragThreshold(start, { clientX: 16, clientY: 10 })).toBe(true);
  });
  it("clips reported rectangles to the viewport", () => {
    const element = elementAt(340, 460, 40, 40);
    expect(collectInteractiveRects([element], 360, 480)).toEqual([
      { x: 340, y: 460, width: 20, height: 20 },
    ]);
  });

  it("drops empty elements", () => {
    expect(collectInteractiveRects([undefined], 360, 480)).toEqual([]);
  });

  it("clips regions that begin outside the top-left edge", () => {
    const element = elementAt(-20, -10, 40, 30);
    expect(collectInteractiveRects([element], 360, 480)).toEqual([
      { x: 0, y: 0, width: 20, height: 20 },
    ]);
  });

  it("drops disconnected portal content", () => {
    const element = {
      isConnected: false,
      getBoundingClientRect: () => ({ x: 20, y: 20, width: 100, height: 100 }),
    } as unknown as Element;
    expect(collectInteractiveRects([element], 360, 480)).toEqual([]);
  });

  it("keeps the silhouette but excludes a closed context menu", () => {
    const silhouette = elementAt(60, 80, 238, 326);
    const menuContent = elementAt(20, 20, 180, 240);

    expect(
      collectPetInteractiveRects(
        {
          silhouette,
          menuContent,
          actionsVisible: false,
          menuOpen: false,
        },
        360,
        480,
      ),
    ).toEqual([{ x: 60, y: 80, width: 238, height: 326 }]);
  });

  it("includes context menu content only while it is open", () => {
    const silhouette = elementAt(60, 80, 238, 326);
    const menuContent = elementAt(20, 20, 180, 240);

    expect(
      collectPetInteractiveRects(
        {
          silhouette,
          menuContent,
          actionsVisible: false,
          menuOpen: true,
        },
        360,
        480,
      ),
    ).toEqual([
      { x: 60, y: 80, width: 238, height: 326 },
      { x: 20, y: 20, width: 180, height: 240 },
    ]);
  });
});
