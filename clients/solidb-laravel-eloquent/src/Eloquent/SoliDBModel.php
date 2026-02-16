<?php

namespace SoliDB\Laravel\Eloquent;

use Illuminate\Database\Eloquent\Model as BaseModel;

abstract class SoliDBModel extends BaseModel
{
    protected $connection = 'solidb';

    protected $keyType = 'string';

    public $incrementing = false;

    protected $primaryKey = '_key';

    public function getTable(): string
    {
        return $this->table ?? $this->getTableFromClass();
    }

    protected function getTableFromClass(): string
    {
        $className = class_basename(static::class);
        
        if (str_ends_with($className, 's')) {
            return strtolower($className);
        }
        
        return strtolower($className) . 's';
    }

    public function getKeyName(): string
    {
        return $this->primaryKey ?? '_key';
    }

    public function getKey(): mixed
    {
        return $this->attributes[$this->getKeyName()] ?? null;
    }

    public function setKey(mixed $key): static
    {
        $this->attributes[$this->getKeyName()] = $key;
        return $this;
    }

    public function getRouteKey(): mixed
    {
        return $this->getKey();
    }

    protected function performInsert(\Illuminate\Database\Eloquent\Builder $query): bool
    {
        if ($this->fireModelEvent('creating') === false) {
            return false;
        }

        $attributes = $this->attributes;
        
        if (empty($attributes[$this->getKeyName()]) && $this->getKeyType() === 'string') {
            $attributes[$this->getKeyName()] = $this->generateKey();
        }

        $query->insert($attributes);

        $this->fireModelEvent('created', false);

        return true;
    }

    protected function performUpdate(\Illuminate\Database\Eloquent\Builder $query): bool
    {
        if ($this->fireModelEvent('updating') === false) {
            return false;
        }

        if ($this->usesTimestamps()) {
            $this->updateTimestamps();
        }

        $dirty = $this->getDirty();

        if (count($dirty) > 0) {
            $query->where($this->getKeyName(), '=', $this->getKey())
                ->update($dirty);
        }

        $this->fireModelEvent('updated', false);

        return true;
    }

    protected function performDelete(): void
    {
        if ($this->fireModelEvent('deleting') === false) {
            return;
        }

        $query = $this->newQuery()->where($this->getKeyName(), '=', $this->getKey());
        $query->delete();

        $this->fireModelEvent('deleted', false);
    }

    protected function generateKey(): string
    {
        return bin2hex(random_bytes(16));
    }

    public function freshTimestamp(): string
    {
        return date('c');
    }

    public function fromDateTime($value): string
    {
        if ($value instanceof \DateTimeInterface) {
            return $value->format('c');
        }
        
        return $value;
    }

    public function toArray(): array
    {
        return $this->attributes;
    }
}
