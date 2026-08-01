import { ref, onMounted, onUnmounted } from 'vue'

export function useResponsive() {
  const isMobile = ref(false)
  const isTablet = ref(false)
  const isCollapse = ref(false)

  const handleResize = () => {
    isMobile.value = window.innerWidth <= 768
    isTablet.value = window.innerWidth > 768 && window.innerWidth <= 1024
    isCollapse.value = window.innerWidth <= 1024
  }

  onMounted(() => {
    handleResize()
    window.addEventListener('resize', handleResize)
  })

  onUnmounted(() => {
    window.removeEventListener('resize', handleResize)
  })

  return {
    isMobile,
    isTablet,
    isCollapse
  }
}
