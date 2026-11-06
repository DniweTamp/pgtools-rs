import * as THREE from "three";

export interface PickResult {
  batchId: string;
  materialId: string;
  distance: number;
}

const sharedRaycaster = new THREE.Raycaster();
const tempBox = new THREE.Box3();

/**
 * Raycasts from normalized device coordinates (`ndcX`, `ndcY` in `[-1, 1]`) against
 * visible batch meshes, using bounding-box early-out before triangle intersection.
 */
export function pickBatchAtNdc(
  ndcX: number,
  ndcY: number,
  camera: THREE.Camera,
  batchMeshes: Map<string, THREE.Mesh>
): PickResult | null {
  sharedRaycaster.setFromCamera(new THREE.Vector2(ndcX, ndcY), camera);
  const candidates: THREE.Mesh[] = [];

  batchMeshes.forEach((mesh) => {
    if (!mesh.visible) return;
    if (!mesh.geometry.boundingBox) {
      mesh.geometry.computeBoundingBox();
    }
    if (mesh.geometry.boundingBox) {
      tempBox.copy(mesh.geometry.boundingBox).applyMatrix4(mesh.matrixWorld);
      if (!sharedRaycaster.ray.intersectsBox(tempBox)) {
        return;
      }
    }
    candidates.push(mesh);
  });

  if (candidates.length === 0) return null;

  const hits = sharedRaycaster.intersectObjects(candidates, false);
  for (const hit of hits) {
    const mesh = hit.object as THREE.Mesh;
    const batchId = mesh.userData?.batchId as string | undefined;
    const materialId = mesh.userData?.materialId as string | undefined;
    if (batchId && materialId) {
      return {
        batchId,
        materialId,
        distance: hit.distance,
      };
    }
  }

  return null;
}
