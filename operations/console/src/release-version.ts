export const MAXIMUM_SEMANTIC_VERSION_LENGTH = 64

export const SEMANTIC_VERSION_INPUT_PATTERN = String.raw`(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-((0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*))?(\+([0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*))?`

export const SEMANTIC_VERSION_HELP = '格式：主版本.次版本.修订版本，例如 2.0.0；不能使用 v 前缀或前导零。'

const semanticVersionPattern = new RegExp(SEMANTIC_VERSION_INPUT_PATTERN)

export function isSemanticVersion(value: string): boolean {
  if (value.length === 0 || value.length > MAXIMUM_SEMANTIC_VERSION_LENGTH) return false
  return semanticVersionPattern.exec(value)?.[0] === value
}
