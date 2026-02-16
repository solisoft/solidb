<?php

return [
    'default' => env('SOLIDB_CONNECTION', 'solidb'),

    'connections' => [
        'solidb' => [
            'driver' => 'solidb',
            'host' => env('SOLIDB_HOST', '127.0.0.1'),
            'port' => env('SOLIDB_PORT', 6745),
            'database' => env('SOLIDB_DATABASE', 'default'),
            'username' => env('SOLIDB_USERNAME', ''),
            'password' => env('SOLIDB_PASSWORD', ''),
            'api_key' => env('SOLIDB_API_KEY', ''),
            'prefix' => env('SOLIDB_PREFIX', ''),
            'options' => [
                'timeout' => env('SOLIDB_TIMEOUT', 30),
            ],
        ],
    ],

    'migrations' => [
        'table' => 'migrations',
        'update_date_on_publish' => true,
    ],
];
