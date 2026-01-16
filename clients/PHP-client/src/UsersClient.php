<?php

namespace SoliDB;

class UsersClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function list(): array
    {
        $res = $this->client->sendCommand('list_users', []);
        return $res['data'] ?? [];
    }

    public function create(string $username, string $password, ?array $roles = null): array
    {
        $args = ['username' => $username, 'password' => $password];
        if ($roles !== null) {
            $args['roles'] = $roles;
        }
        $res = $this->client->sendCommand('create_user', $args);
        return $res['data'] ?? [];
    }

    public function get(string $username): array
    {
        $res = $this->client->sendCommand('get_user', ['username' => $username]);
        return $res['data'] ?? [];
    }

    public function delete(string $username): void
    {
        $this->client->sendCommand('delete_user', ['username' => $username]);
    }

    public function getRoles(string $username): array
    {
        $res = $this->client->sendCommand('get_user_roles', ['username' => $username]);
        return $res['data'] ?? [];
    }

    public function assignRole(string $username, string $role, ?string $database = null): void
    {
        $args = ['username' => $username, 'role' => $role];
        if ($database !== null) {
            $args['database'] = $database;
        }
        $this->client->sendCommand('assign_role', $args);
    }

    public function revokeRole(string $username, string $role, ?string $database = null): void
    {
        $args = ['username' => $username, 'role' => $role];
        if ($database !== null) {
            $args['database'] = $database;
        }
        $this->client->sendCommand('revoke_role', $args);
    }

    public function me(): array
    {
        $res = $this->client->sendCommand('get_current_user', []);
        return $res['data'] ?? [];
    }

    public function myPermissions(): array
    {
        $res = $this->client->sendCommand('get_my_permissions', []);
        return $res['data'] ?? [];
    }

    public function changePassword(string $username, string $oldPassword, string $newPassword): void
    {
        $this->client->sendCommand('change_password', [
            'username' => $username,
            'old_password' => $oldPassword,
            'new_password' => $newPassword
        ]);
    }
}
