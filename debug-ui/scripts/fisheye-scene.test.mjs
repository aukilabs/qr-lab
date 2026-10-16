import { describe, it, expect } from "vitest";
import { sceneGeometry, renderScene } from "./fisheye-scene";

const pose = { resolution: 640, side: 224, phase: 0, dim: 29, tilt: 0, azimuth: 0, position: "center", margin: null };
describe("sweep reference renderer", () => {
  it("uses integer-centered pixels and exact independent pinhole geometry", () => {
    const g = sceneGeometry(pose);
    expect(g.corners).toEqual([[207.5,207.5],[431.5,207.5],[431.5,431.5],[207.5,431.5]]);
    expect(g.project(0.5,0.5)).toEqual([319.5,319.5]);
    expect(g.incidence).toBe(0);
    const m = g.inverse;
    for (const [u,v] of [[0,0],[1,0],[1,1],[0,1],[.3,.7]]) {
      const [x,y] = g.project(u,v), w = m[2]*x+m[5]*y+m[8];
      expect((m[0]*x+m[3]*y+m[6])/w).toBeCloseTo(u,12);
      expect((m[1]*x+m[4]*y+m[7])/w).toBeCloseTo(v,12);
    }
  });
  it.each([0,20,55,75])("places the quiet zone at the requested edge for tilt %s", tilt => {
    for (const azimuth of [0,90,180,270]) for (const position of ["left","right","top","bottom","tl","tr","br","bl"]) {
      const g = sceneGeometry({...pose,tilt,azimuth,position,margin:4});
      const margin = Math.min(...g.quietCorners.flatMap(([x,y])=>[x+.5,y+.5,639.5-x,639.5-y]));
      expect(margin).toBeCloseTo(4,10);
    }
  });
  it("culls the back of a plane and renders an opaque quiet zone", () => {
    const words = new Array(29).fill(0xffffffff);
    const rgba = renderScene(pose, words, 2);
    const at = (x,y) => [...rgba.slice(4*(y*640+x),4*(y*640+x)+4)];
    expect(at(319,319)).toEqual([0,0,0,255]);
    expect(at(190,319)).toEqual([255,255,255,255]);
    expect(at(10,10)).toEqual([5,7,13,255]);
    const back = {...pose,tilt:82,position:"left",margin:4};
    expect(sceneGeometry(back).frontFacing).toBe(false);
    const pixels = renderScene(back,words);
    expect(pixels.every((v,i)=>v===[5,7,13,255][i%4])).toBe(true);
  });
});
