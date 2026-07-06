<?php

declare(strict_types=1);

$token = 'dev-token';
$fileUrl = 'https://raw.githubusercontent.com/vergil-lai/print-bridge-jssdk/main/examples/assets/printbridge-a4-sample.jpg';
$format = 'image';

if (parse_url($_SERVER['REQUEST_URI'] ?? '/', PHP_URL_PATH) !== '/print-task') {
    send_json(404, ['error' => 'not_found']);
}

if (($_SERVER['HTTP_AUTHORIZATION'] ?? '') !== 'Bearer ' . $token) {
    send_json(401, ['error' => 'unauthorized']);
}

$method = $_SERVER['REQUEST_METHOD'] ?? 'GET';

if (($_SERVER['HTTP_X_PRINTBRIDGE_TEST'] ?? '') === 'true') {
    if ($method === 'GET' || $method === 'POST') {
        http_response_code(204);
        exit;
    }

    send_json(405, ['error' => 'method_not_allowed']);
}

if ($method === 'GET') {
    send_json(200, [
        'type' => 'print',
        'request_id' => 'REQ-PHP-IMAGE',
        'job_id' => 'JOB-PHP-IMAGE',
        'format' => $format,
        'file_url' => $fileUrl,
        'copies' => 1,
    ]);
}

if ($method === 'POST') {
    $report = json_decode(file_get_contents('php://input') ?: 'null', true);
    error_log('PrintBridge status report: ' . json_encode($report, JSON_UNESCAPED_SLASHES));
    http_response_code(204);
    exit;
}

send_json(405, ['error' => 'method_not_allowed']);

function send_json(int $statusCode, array $body): never
{
    http_response_code($statusCode);
    header('Content-Type: application/json; charset=utf-8');
    echo json_encode($body, JSON_UNESCAPED_SLASHES);
    exit;
}
