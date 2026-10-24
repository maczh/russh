import i18n from './index'

export function getSystemLanguage(languages = navigator.languages): 'zh-CN' | 'en-US' {
  return languages.some((language) => language.toLowerCase().startsWith('zh')) ? 'zh-CN' : 'en-US'
}

export function resolveAppLanguage(language: 'system' | 'zh-CN' | 'en-US'): 'zh-CN' | 'en-US' {
  return language === 'system' ? getSystemLanguage() : language
}

export function getResolvedLocale(): string {
  const language = i18n.resolvedLanguage ?? i18n.language
  if (language === 'zh-CN' || language === 'en-US') {
    return language
  }

  return getSystemLanguage()
}

export function formatDateTime(
  value: Date | string | number,
  options?: Intl.DateTimeFormatOptions
): string {
  const date = toDate(value)
  const hasComponentOptions = options && Object.keys(options).some((key) =>
    ['weekday', 'year', 'month', 'day', 'dayPeriod', 'hour', 'minute', 'second', 'fractionalSecondDigits', 'timeZoneName'].includes(key)
  )
  const formatterOptions: Intl.DateTimeFormatOptions = hasComponentOptions
    ? options
    : { dateStyle: 'medium', timeStyle: 'short', ...options }
  return new Intl.DateTimeFormat(getResolvedLocale(), formatterOptions).format(date)
}

export function formatTime(
  value: Date | string | number,
  options?: Intl.DateTimeFormatOptions
): string {
  const date = toDate(value)
  const hasComponentOptions = options && Object.keys(options).some((key) =>
    ['weekday', 'year', 'month', 'day', 'dayPeriod', 'hour', 'minute', 'second', 'fractionalSecondDigits', 'timeZoneName'].includes(key)
  )
  const formatterOptions: Intl.DateTimeFormatOptions = hasComponentOptions ? options : { timeStyle: 'medium', ...options }
  return new Intl.DateTimeFormat(getResolvedLocale(), formatterOptions).format(date)
}

function toDate(value: Date | string | number): Date {
  if (value instanceof Date) {
    return Number.isFinite(value.getTime()) ? value : new Date(0)
  }

  if (typeof value === 'number' || (typeof value === 'string' && /^\d+(?:\.\d+)?$/.test(value))) {
    const numericValue = Number(value)
    const milliseconds = numericValue < 1_000_000_000_000 ? numericValue * 1000 : numericValue
    return Number.isFinite(milliseconds) ? new Date(milliseconds) : new Date(0)
  }

  const date = new Date(value)
  return Number.isFinite(date.getTime()) ? date : new Date(0)
}

const FILE_SIZE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'] as const

export function formatFileSize(value: number): string {
  const safeValue = Number.isFinite(value) && value > 0 ? value : 0

  if (safeValue === 0) {
    return '0 B'
  }

  let size = safeValue
  let unitIndex = 0

  while (size >= 1024 && unitIndex < FILE_SIZE_UNITS.length - 1) {
    size /= 1024
    unitIndex += 1
  }

  if (unitIndex === 0) {
    return `${new Intl.NumberFormat(getResolvedLocale(), {
      maximumFractionDigits: 0
    }).format(size)} ${FILE_SIZE_UNITS[unitIndex]}`
  }

  return `${new Intl.NumberFormat(getResolvedLocale(), {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2
  }).format(size)} ${FILE_SIZE_UNITS[unitIndex]}`
}
