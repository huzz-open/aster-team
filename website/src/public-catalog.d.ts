declare module 'virtual:aster-public-catalog' {
  const catalog: import('../shared/generated/contracts').PublicCatalog | null
  export const identity: { revision: string; sha256: string; environment: 'local' | 'production'; path: string } | null
  export default catalog
}
