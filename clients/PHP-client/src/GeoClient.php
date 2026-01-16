<?php

namespace SoliDB;

class GeoClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function createIndex(string $collection, string $name, array $fields, ?bool $geoJson = null): array
    {
        $args = [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'name' => $name,
            'fields' => $fields
        ];
        if ($geoJson !== null) {
            $args['geo_json'] = $geoJson;
        }
        $res = $this->client->sendCommand('create_geo_index', $args);
        return $res['data'] ?? [];
    }

    public function listIndexes(string $collection): array
    {
        $res = $this->client->sendCommand('list_geo_indexes', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
        return $res['data'] ?? [];
    }

    public function deleteIndex(string $collection, string $indexName): void
    {
        $this->client->sendCommand('delete_geo_index', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'index_name' => $indexName
        ]);
    }

    public function near(string $collection, float $latitude, float $longitude, float $radius, ?int $limit = null): array
    {
        $args = [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'latitude' => $latitude,
            'longitude' => $longitude,
            'radius' => $radius
        ];
        if ($limit !== null) {
            $args['limit'] = $limit;
        }
        $res = $this->client->sendCommand('geo_near', $args);
        return $res['data'] ?? [];
    }

    public function within(string $collection, array $geometry): array
    {
        $res = $this->client->sendCommand('geo_within', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'geometry' => $geometry
        ]);
        return $res['data'] ?? [];
    }

    public function distance(float $lat1, float $lon1, float $lat2, float $lon2): float
    {
        $res = $this->client->sendCommand('geo_distance', [
            'lat1' => $lat1,
            'lon1' => $lon1,
            'lat2' => $lat2,
            'lon2' => $lon2
        ]);
        return $res['data'] ?? 0.0;
    }

    public function intersects(string $collection, array $geometry): array
    {
        $res = $this->client->sendCommand('geo_intersects', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'geometry' => $geometry
        ]);
        return $res['data'] ?? [];
    }
}
