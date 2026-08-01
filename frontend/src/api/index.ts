import { invoke } from '@tauri-apps/api/core'

export interface BridgeResponse {
  data?: unknown
  [key: string]: unknown
}

function normalizePath(path: string): string {
  return path.replace(/\/+/g, '/').replace(/\/$/, '') || '/'
}

function normalizeInvokeError(error: unknown): Error {
  if (typeof error === 'string') return new Error(error)
  if (error && typeof error === 'object') {
    const value = error as { message?: unknown }
    if (typeof value.message === 'string') return new Error(value.message)
  }
  return new Error('Tauri command failed without an error message')
}

async function call<T>(method: string, path: string, params: Record<string, unknown>, payload: unknown): Promise<{ data: T }> {
  try {
    const data = await invoke<T>('invoke_api', { request: { method, path: normalizePath(path), params, payload } })
    return { data }
  } catch (error) {
    throw normalizeInvokeError(error)
  }
}

const api = {
  async get<T = any>(path: string, config?: { params?: Record<string, unknown> }): Promise<{ data: T }> {
    return call<T>('GET', path, config?.params ?? {}, {})
  },
  async post<T = any>(path: string, payload?: unknown, config?: { params?: Record<string, unknown> }): Promise<{ data: T }> {
    return call<T>('POST', path, config?.params ?? {}, payload ?? {})
  },
  async put<T = any>(path: string, payload?: unknown): Promise<{ data: T }> {
    return call<T>('PUT', path, {}, payload ?? {})
  },
  async delete<T = any>(path: string, config?: { params?: Record<string, unknown> }): Promise<{ data: T }> {
    return call<T>('DELETE', path, config?.params ?? {}, {})
  }
}

export default api
