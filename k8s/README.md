# SoliDB Kubernetes Deployment

Deploy SoliDB on Kubernetes as a single instance or a 3-node cluster.

## Prerequisites

- Kubernetes 1.21+
- kubectl configured
- Storage provisioner (for PVCs)

## Quick Start

### Single Node

```bash
# Create namespace and config
kubectl apply -f namespace.yaml
kubectl apply -f configmap.yaml
kubectl apply -f secret.yaml

# Deploy single node
kubectl apply -f single/

# Verify
kubectl -n solidb get pods
kubectl -n solidb port-forward svc/solidb 6745:6745
curl http://localhost:6745/_api/health
```

### Cluster (3 nodes)

```bash
# Generate keyfile for cluster authentication
openssl rand -hex 32 > keyfile.txt

# Update secret with keyfile
kubectl create secret generic solidb-secret \
  --namespace solidb \
  --from-file=keyfile=keyfile.txt \
  --dry-run=client -o yaml | kubectl apply -f -

# Deploy cluster
kubectl apply -f namespace.yaml
kubectl apply -f configmap.yaml
kubectl apply -f cluster/

# Verify all nodes are running
kubectl -n solidb get pods -w

# Check cluster health
kubectl -n solidb port-forward svc/solidb 6745:6745
curl http://localhost:6745/_api/health
```

## Architecture

### Single Node
```
┌─────────────────────────────────────┐
│  Deployment (1 replica)             │
│  └─ Pod: solidb                     │
│      └─ PVC: solidb-data (10Gi)     │
└─────────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────┐
│  Service: solidb (ClusterIP)        │
│  Port: 6745                         │
└─────────────────────────────────────┘
```

### Cluster
```
┌─────────────────────────────────────────────────────────────┐
│  StatefulSet (3 replicas)                                   │
│  ├─ Pod: solidb-0  ──► PVC: data-solidb-0                   │
│  ├─ Pod: solidb-1  ──► PVC: data-solidb-1                   │
│  └─ Pod: solidb-2  ──► PVC: data-solidb-2                   │
└─────────────────────────────────────────────────────────────┘
         │                              │
         ▼                              ▼
┌─────────────────────┐    ┌─────────────────────────────────┐
│  Service: solidb    │    │  Headless: solidb-headless      │
│  (ClusterIP)        │    │  DNS peer discovery:            │
│  Client connections │    │  solidb-0.solidb-headless...    │
└─────────────────────┘    └─────────────────────────────────┘
```

## Configuration

### ConfigMap

| Key | Default | Description |
|-----|---------|-------------|
| SOLIDB_PORT | 6745 | HTTP server port |
| SOLIDB_LOG_LEVEL | info | Log verbosity |
| RUST_LOG | solidb=info | Rust logging filter |

### Secret

| Key | Description |
|-----|-------------|
| keyfile | Cluster authentication key (required for cluster mode) |
| admin-password | Optional admin password override |

## Scaling

```bash
# Scale cluster (minimum 3 for quorum)
kubectl -n solidb scale statefulset solidb --replicas=5

# Note: Update REPLICAS in statefulset init container if scaling beyond 3
```

## Monitoring

Pods expose Prometheus metrics at `/metrics`. Annotations are pre-configured:

```yaml
prometheus.io/scrape: "true"
prometheus.io/port: "6745"
prometheus.io/path: "/metrics"
```

## Cleanup

```bash
# Delete single node
kubectl delete -f single/

# Delete cluster
kubectl delete -f cluster/

# Delete namespace (removes everything)
kubectl delete namespace solidb
```
