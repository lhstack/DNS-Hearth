import { defineStore } from 'pinia'
import { ref } from 'vue'

export const useAuthStore = defineStore('auth', () => {
  const username = ref<string | null>('local')
  const isAuthenticated = ref(true)
  function logout() { isAuthenticated.value = false }
  return { username, isAuthenticated, logout }
})
