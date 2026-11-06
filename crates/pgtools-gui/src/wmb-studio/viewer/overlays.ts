// [wmb-studio BEGIN phase2]
import * as THREE from "three";
import { BoneDesc } from "../contract";

/**
 * Builds the ground grid (`y = 0`) and subtle XYZ axis lines at the origin.
 */
export function createGridAndAxesOverlay(theme: string): THREE.Group {
  const group = new THREE.Group();
  group.name = "wmb-grid-overlay";

  const isLight = theme === "light";
  const centerColor = isLight ? 0x94a3b8 : 0x334155;
  const gridColor = isLight ? 0xcbd5e1 : 0x1e293b;

  const grid = new THREE.GridHelper(6, 24, centerColor, gridColor);
  grid.renderOrder = -10;
  group.add(grid);

  // Subtle origin axis hint (+X red, +Y green, +Z blue)
  const axisPositions = new Float32Array([
    0, 0.002, 0, 0.6, 0.002, 0,
    0, 0, 0, 0, 0.6, 0,
    0, 0.002, 0, 0, 0.002, 0.6,
  ]);
  const axisColors = new Float32Array([
    0.92, 0.28, 0.28, 0.92, 0.28, 0.28,
    0.22, 0.82, 0.38, 0.22, 0.82, 0.38,
    0.25, 0.55, 0.96, 0.25, 0.55, 0.96,
  ]);
  const axisGeom = new THREE.BufferGeometry();
  axisGeom.setAttribute("position", new THREE.BufferAttribute(axisPositions, 3));
  axisGeom.setAttribute("color", new THREE.BufferAttribute(axisColors, 3));
  const axisMat = new THREE.LineBasicMaterial({
    vertexColors: true,
    transparent: true,
    opacity: 0.85,
    depthWrite: false,
  });
  const axisLines = new THREE.LineSegments(axisGeom, axisMat);
  axisLines.renderOrder = -9;
  group.add(axisLines);

  return group;
}

/**
 * Builds or updates the 3D Skeleton overlay (`bones` lines + joint markers),
 * highlighting any selected bone IDs in `#facc15` yellow.
 */
export function buildSkeletonOverlay(
  bones: BoneDesc[],
  selectedBoneIds: Set<number>
): THREE.Group {
  const group = new THREE.Group();
  group.name = "wmb-skeleton-overlay";
  group.renderOrder = 999;

  if (bones.length === 0) return group;

  const byGlobalId = new Map<number, BoneDesc>();
  const byIndex = new Map<number, BoneDesc>();
  bones.forEach((b, idx) => {
    byGlobalId.set(b.globalId, b);
    byIndex.set(idx, b);
  });

  const linePositions: number[] = [];
  const lineColors: number[] = [];
  const jointPositions: number[] = [];
  const jointColors: number[] = [];

  for (const b of bones) {
    const wp = b.worldPosition;
    if (wp.length < 3) continue;
    const isSel = selectedBoneIds.has(b.globalId);

    jointPositions.push(wp[0], wp[1], wp[2]);
    if (isSel) {
      jointColors.push(0.98, 0.8, 0.08); // #facc15
    } else {
      jointColors.push(0.22, 0.78, 0.95); // cyan joint
    }

    if (b.parent >= 0) {
      const parentBone =
        byIndex.get(b.parent) ?? byGlobalId.get(b.parent);
      if (parentBone && parentBone.worldPosition.length >= 3) {
        const pp = parentBone.worldPosition;
        const parentSel = selectedBoneIds.has(parentBone.globalId);
        linePositions.push(pp[0], pp[1], pp[2], wp[0], wp[1], wp[2]);
        if (isSel || parentSel) {
          lineColors.push(0.98, 0.8, 0.08, 0.98, 0.8, 0.08);
        } else {
          lineColors.push(0.18, 0.65, 0.88, 0.38, 0.85, 0.98);
        }
      }
    }
  }

  if (linePositions.length > 0) {
    const lineGeom = new THREE.BufferGeometry();
    lineGeom.setAttribute(
      "position",
      new THREE.Float32BufferAttribute(linePositions, 3)
    );
    lineGeom.setAttribute(
      "color",
      new THREE.Float32BufferAttribute(lineColors, 3)
    );
    const lineMat = new THREE.LineBasicMaterial({
      vertexColors: true,
      depthTest: false,
      depthWrite: false,
      transparent: true,
      opacity: 0.9,
    });
    const lines = new THREE.LineSegments(lineGeom, lineMat);
    lines.renderOrder = 999;
    group.add(lines);
  }

  if (jointPositions.length > 0) {
    const ptGeom = new THREE.BufferGeometry();
    ptGeom.setAttribute(
      "position",
      new THREE.Float32BufferAttribute(jointPositions, 3)
    );
    ptGeom.setAttribute(
      "color",
      new THREE.Float32BufferAttribute(jointColors, 3)
    );
    const ptMat = new THREE.PointsMaterial({
      size: 6,
      sizeAttenuation: false,
      vertexColors: true,
      depthTest: false,
      depthWrite: false,
    });
    const pts = new THREE.Points(ptGeom, ptMat);
    pts.renderOrder = 1000;
    group.add(pts);
  }

  return group;
}

export function disposeObjectGroup(group: THREE.Group): void {
  group.traverse((obj: THREE.Object3D) => {
    if (
      obj instanceof THREE.Mesh ||
      obj instanceof THREE.LineSegments ||
      obj instanceof THREE.Points
    ) {
      obj.geometry?.dispose();
      if (Array.isArray(obj.material)) {
        obj.material.forEach((m: THREE.Material) => m.dispose());
      } else {
        obj.material?.dispose();
      }
    }
  });
  group.clear();
}
// [wmb-studio END]
