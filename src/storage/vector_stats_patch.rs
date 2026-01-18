
    /// Get index statistics
    pub fn stats(&self) -> VectorIndexStats {
        let (quantization, compression_ratio, memory_bytes) = 
            if let Some(stats) = self.quantization_stats() {
                (stats.type_str(), stats.compression_ratio, stats.memory_bytes)
            } else {
                (VectorQuantization::None, 1.0, 0)
            };

        VectorIndexStats {
            name: self.config.name.clone(),
            field: self.config.field.clone(),
            dimension: self.config.dimension,
            metric: self.config.metric,
            m: self.config.m,
            ef_construction: self.config.ef_construction,
            indexed_vectors: self.len(),
            quantization,
            memory_bytes,
            compression_ratio,
        }
    }
