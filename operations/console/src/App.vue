<script setup lang="ts">
import { AToastHost, configureToastDefaultDuration } from '@aster/ui'
import SessionExpiredDialog from './components/SessionExpiredDialog.vue'

configureToastDefaultDuration(2_000)
</script>

<template><RouterView /><AToastHost /><SessionExpiredDialog /></template>
