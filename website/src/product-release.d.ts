declare module 'virtual:aster-product-release' {
  export const downloadConfig: { url: string; version: string | null; platform: string | null; filename: string | null } | null
  type ProductRelease = {
    environment: 'local' | 'production'
    version: string
    platform: 'linux-amd64'
    artifact: { name: string; sha256: string; size_bytes: number; platform: 'linux-amd64'; url: string }
    documents: { user_manual: string; linux_guide: string; checksums: string }
    support_manifest: { sha256: string; release_manifest_sha256: string; release_key_id: string }
    free_plan: { plan_id: string; plan_version: number; license_id: string; license_sha256: string }
    windows?: {
      environment: 'local' | 'production'
      version: string
      platform: 'windows-amd64'
      artifact: { name: string; sha256: string; size_bytes: number; platform: 'windows-amd64'; channel: 'experimental'; url: string }
      documents: { user_manual: string; windows_guide: string; checksums: string }
      support_manifest: { sha256: string; release_manifest_sha256: string; release_key_id: string }
      free_plan: { plan_id: string; plan_version: number; license_id: string; license_sha256: string }
    }
  }
  const release: ProductRelease | null
  export default release
}
