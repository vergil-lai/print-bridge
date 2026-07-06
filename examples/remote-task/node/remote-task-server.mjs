import { createServer } from 'node:http';

const port = 18080;
const token = 'dev-token';
const fileUrl =
  'https://raw.githubusercontent.com/vergil-lai/print-bridge-jssdk/main/examples/assets/printbridge-a4-sample.pdf';

const server = createServer(async (request, response) => {
  if (request.url !== '/print-task') {
    sendJson(response, 404, { error: 'not_found' });
    return;
  }

  if (!isAuthorized(request)) {
    sendJson(response, 401, { error: 'unauthorized' });
    return;
  }

  if (request.headers['x-printbridge-test'] === 'true') {
    handleConnectionTest(request, response);
    return;
  }

  if (request.method === 'GET') {
    sendJson(response, 200, {
      type: 'print',
      request_id: 'REQ-NODE-PDF',
      job_id: 'JOB-NODE-PDF',
      format: 'pdf',
      file_url: fileUrl,
      copies: 1,
    });
    return;
  }

  if (request.method === 'POST') {
    const report = await readJson(request);
    console.log('PrintBridge status report:', report);
    response.writeHead(204);
    response.end();
    return;
  }

  sendJson(response, 405, { error: 'method_not_allowed' });
});

server.listen(port, '127.0.0.1', () => {
  console.log(`Remote task example listening on http://127.0.0.1:${port}/print-task`);
  console.log(`Bearer token: ${token}`);
});

function handleConnectionTest(request, response) {
  if (request.method === 'GET') {
    response.writeHead(204);
    response.end();
    return;
  }

  if (request.method === 'POST') {
    response.writeHead(204);
    response.end();
    return;
  }

  sendJson(response, 405, { error: 'method_not_allowed' });
}

function isAuthorized(request) {
  return request.headers.authorization === `Bearer ${token}`;
}

function sendJson(response, statusCode, body) {
  response.writeHead(statusCode, { 'content-type': 'application/json; charset=utf-8' });
  response.end(JSON.stringify(body));
}

async function readJson(request) {
  const chunks = [];
  for await (const chunk of request) {
    chunks.push(chunk);
  }

  const body = Buffer.concat(chunks).toString('utf8');
  return body ? JSON.parse(body) : null;
}
