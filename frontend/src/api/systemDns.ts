import { invoke } from '@tauri-apps/api/core'

export interface NetworkDnsBinding {
  service: string
  enabled: boolean
  preset_servers: string[]
}

export interface SystemDnsConfig {
  bindings: NetworkDnsBinding[]
  check_interval_secs: number
}

export interface NetworkServiceDns {
  service: string
  current_servers: string[]
  enabled: boolean
  preset_servers: string[]
  matches_preset: boolean
}

export interface UdpListenerConfiguration {
  enabled: boolean
  bind_address: string
  port: number
}

export interface HomeConfiguration {
  system_dns: SystemDnsConfig
  udp_enabled: boolean
  udp_bind_address: string
  udp_port: number
}

export interface HomeConfigurationResult {
  system_dns: SystemDnsConfig
  udp_listener: UdpListenerConfiguration
  services: NetworkServiceDns[]
}

export async function getSystemDnsConfig(): Promise<SystemDnsConfig> {
  return invoke<SystemDnsConfig>('get_system_dns_config')
}

export async function getNetworkServices(): Promise<NetworkServiceDns[]> {
  return invoke<NetworkServiceDns[]>('get_network_services')
}

export async function updateSystemDnsConfig(config: SystemDnsConfig): Promise<SystemDnsConfig> {
  return invoke<SystemDnsConfig>('update_system_dns_config', { config })
}

export async function enforceSystemDns(): Promise<NetworkServiceDns[]> {
  return invoke<NetworkServiceDns[]>('enforce_system_dns')
}

export async function applyHomeConfiguration(configuration: HomeConfiguration): Promise<HomeConfigurationResult> {
  return invoke<HomeConfigurationResult>('apply_home_configuration', { configuration })
}
