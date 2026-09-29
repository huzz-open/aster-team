<script setup lang="ts">
import SiteIcon from './SiteIcon.vue'
import PrivateArchitectureFrame from './PrivateArchitectureFrame.vue'
import PrivateArchitectureConnectors from './PrivateArchitectureConnectors.vue'
import PrivatePortalCard from './PrivatePortalCard.vue'
import PrivateNodeCard from './PrivateNodeCard.vue'
import PrivateLicenseArt from './PrivateLicenseArt.vue'
import PrivateInsetCard from './PrivateInsetCard.vue'
defineProps<{ labels: { environment: string; user: string; admin: string; control: string; data: string; runner: string; upstream: string; upstreamAuth: string; inference: string; license: string } }>()
</script>

<template>
<div class="private-architecture-scroll" tabindex="0" :aria-label="labels.environment" data-lenis-prevent>
<div class="private-architecture" role="img" :aria-label="labels.environment">
          <PrivateArchitectureFrame />
          <PrivateArchitectureConnectors />
          <span class="private-environment-title"><SiteIcon name="shield" :size="18" />{{ labels.environment }}</span>
          <article class="private-layer-node private-user-node"><PrivatePortalCard variant="user" /><span class="private-portal-icon"><SiteIcon name="user" :size="28" /></span><div class="private-layer-copy"><strong>{{ labels.user }}</strong></div></article>
          <article class="private-layer-node private-admin-node"><PrivatePortalCard variant="admin" /><span class="private-portal-icon"><SiteIcon name="settings" :size="28" /></span><div class="private-layer-copy"><strong>{{ labels.admin }}</strong></div></article>

          <article class="private-layer-node private-control-node">
            <PrivateNodeCard variant="control" />
            <span class="private-node-icon private-control-icon"><SiteIcon name="sliders" :size="34" /></span>
            <div class="private-layer-copy private-control-copy"><strong>{{ labels.control }}</strong></div>
            <span class="private-license-chip"><PrivateLicenseArt /><b>{{ labels.license }}</b></span>
          </article>

          <article class="private-layer-node private-data-node"><PrivateNodeCard variant="data" /><span class="private-node-icon private-data-icon"><SiteIcon name="database" :size="40" /></span><div class="private-layer-copy"><strong>{{ labels.data }}</strong></div></article>
          <article class="private-layer-node private-runner-node"><PrivateNodeCard variant="runner" /><span class="private-node-icon private-runner-icon"><SiteIcon name="server" :size="34" /></span><div class="private-layer-copy"><strong>{{ labels.runner }}</strong></div></article>
          <article class="private-layer-node private-upstream-node"><PrivateNodeCard variant="upstream" /><div class="private-layer-copy"><strong>{{ labels.upstream }}</strong></div><div class="private-upstream-details"><span><PrivateInsetCard /><i class="private-capability-icon"><SiteIcon name="user" :size="28" /></i><b>{{ labels.upstreamAuth }}</b></span><span><PrivateInsetCard /><i class="private-capability-icon"><SiteIcon name="model" :size="28" /></i><b>{{ labels.inference }}</b></span></div></article>
        </div>
</div>
</template>

<style src="./private-architecture.css"></style>
