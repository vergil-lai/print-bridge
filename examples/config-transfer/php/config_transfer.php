<?php

const ENCRYPTED_FORMAT = 'printbridge-config-encrypted';
const PAYLOAD_FORMAT = 'printbridge-config';
const TRANSFER_VERSION = 1;
const SALT_BYTES = 16;
const NONCE_BYTES = 12;
const KEY_BYTES = 32;
const ARGON2_MEMORY_KIB = 19456;
const ARGON2_ITERATIONS = 2;
const ARGON2_PARALLELISM = 1;
const GCM_TAG_BYTES = 16;
const EXPECTED_PAYLOAD = 'OTX3vkYug76bv335qmWdp85pbgu85QfwarlnqhxGoV0U+4sRez0dlwWy+5eIe597KLRqdHg7XJVbjLds/mXROcLHLhTJrJJ+DWpB2Xc6BX2sKii+bziOsb8akhUwxqo=';

function encryptConfigFile(array $payload, string $password, array $options = []): array
{
    $salt = $options['salt'] ?? random_bytes(SALT_BYTES);
    $nonce = $options['nonce'] ?? random_bytes(NONCE_BYTES);

    if (strlen($salt) !== SALT_BYTES || strlen($nonce) !== NONCE_BYTES) {
        throw new RuntimeException('Invalid salt or nonce length.');
    }

    $key = sodium_crypto_pwhash(
        KEY_BYTES,
        $password,
        $salt,
        ARGON2_ITERATIONS,
        ARGON2_MEMORY_KIB * 1024,
        SODIUM_CRYPTO_PWHASH_ALG_ARGON2ID13
    );

    $plaintext = json_encode($payload, JSON_UNESCAPED_SLASHES);
    if ($plaintext === false) {
        throw new RuntimeException('Failed to encode payload.');
    }

    $tag = '';
    $ciphertext = openssl_encrypt(
        $plaintext,
        'aes-256-gcm',
        $key,
        OPENSSL_RAW_DATA,
        $nonce,
        $tag,
        '',
        GCM_TAG_BYTES
    );
    sodium_memzero($key);

    if ($ciphertext === false || strlen($tag) !== GCM_TAG_BYTES) {
        throw new RuntimeException('Failed to encrypt payload.');
    }

    return [
        'format' => ENCRYPTED_FORMAT,
        'version' => TRANSFER_VERSION,
        'crypto' => [
            'kdf' => 'argon2id13',
            'memory_kib' => ARGON2_MEMORY_KIB,
            'iterations' => ARGON2_ITERATIONS,
            'parallelism' => ARGON2_PARALLELISM,
            'cipher' => 'aes-256-gcm',
            'tag_bytes' => GCM_TAG_BYTES,
            'salt' => base64_encode($salt),
            'nonce' => base64_encode($nonce),
        ],
        'payload' => base64_encode($ciphertext . $tag),
    ];
}

function decryptConfigFile(array $file, string $password): array
{
    validateEnvelope($file);

    $salt = base64_decode($file['crypto']['salt'], true);
    $nonce = base64_decode($file['crypto']['nonce'], true);
    $encryptedPayload = base64_decode($file['payload'], true);

    if (
        $salt === false ||
        $nonce === false ||
        $encryptedPayload === false ||
        strlen($salt) !== SALT_BYTES ||
        strlen($nonce) !== NONCE_BYTES ||
        strlen($encryptedPayload) < GCM_TAG_BYTES
    ) {
        throw new RuntimeException('Invalid encrypted config payload.');
    }

    $ciphertext = substr($encryptedPayload, 0, -GCM_TAG_BYTES);
    $tag = substr($encryptedPayload, -GCM_TAG_BYTES);
    $key = sodium_crypto_pwhash(
        KEY_BYTES,
        $password,
        $salt,
        ARGON2_ITERATIONS,
        ARGON2_MEMORY_KIB * 1024,
        SODIUM_CRYPTO_PWHASH_ALG_ARGON2ID13
    );

    $plaintext = openssl_decrypt(
        $ciphertext,
        'aes-256-gcm',
        $key,
        OPENSSL_RAW_DATA,
        $nonce,
        $tag
    );
    sodium_memzero($key);

    if ($plaintext === false) {
        throw new RuntimeException('Invalid password or encrypted config payload.');
    }

    $payload = json_decode($plaintext, true);
    if (!is_array($payload) || ($payload['format'] ?? null) !== PAYLOAD_FORMAT || ($payload['version'] ?? null) !== TRANSFER_VERSION) {
        throw new RuntimeException('Invalid decrypted config payload.');
    }

    return $payload;
}

function validateEnvelope(array $file): void
{
    $crypto = $file['crypto'] ?? null;
    if (
        ($file['format'] ?? null) !== ENCRYPTED_FORMAT ||
        ($file['version'] ?? null) !== TRANSFER_VERSION ||
        !is_array($crypto) ||
        ($crypto['kdf'] ?? null) !== 'argon2id13' ||
        ($crypto['memory_kib'] ?? null) !== ARGON2_MEMORY_KIB ||
        ($crypto['iterations'] ?? null) !== ARGON2_ITERATIONS ||
        ($crypto['parallelism'] ?? null) !== ARGON2_PARALLELISM ||
        ($crypto['cipher'] ?? null) !== 'aes-256-gcm' ||
        ($crypto['tag_bytes'] ?? null) !== GCM_TAG_BYTES
    ) {
        throw new RuntimeException('Invalid encrypted config file.');
    }
}

function verify(): void
{
    $payload = [
        'format' => PAYLOAD_FORMAT,
        'version' => TRANSFER_VERSION,
        'config' => [
            'service' => [
                'port' => 17890,
            ],
        ],
    ];

    $file = encryptConfigFile($payload, 'test-password', [
        'salt' => hex2bin('000102030405060708090a0b0c0d0e0f'),
        'nonce' => hex2bin('101112131415161718191a1b'),
    ]);

    if ($file['payload'] !== EXPECTED_PAYLOAD) {
        throw new RuntimeException('Encrypted payload does not match expected vector.');
    }

    $decrypted = decryptConfigFile($file, 'test-password');
    if (($decrypted['config']['service']['port'] ?? null) !== 17890) {
        throw new RuntimeException('Decrypted payload did not round-trip.');
    }

    echo json_encode($file, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES) . PHP_EOL;
}

if (PHP_SAPI === 'cli' && ($argv[1] ?? null) === 'verify') {
    verify();
}
