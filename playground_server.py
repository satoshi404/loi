#!/usr/bin/env python3
"""
Simple HTTP server for loi playground.
Serves static files from site/ and handles /api/compile endpoint.
"""
import http.server
import socketserver
import json
import subprocess
import tempfile
import os
import sys
import urllib.parse
import threading
import time

PORT = 8080
SITE_DIR = os.path.join(os.path.dirname(__file__), 'site')
LOI_BIN = os.path.join(os.path.dirname(__file__), 'loi')

class PlaygroundHandler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=SITE_DIR, **kwargs)
    
    def do_POST(self):
        if self.path == '/api/compile':
            self.handle_compile()
        else:
            self.send_error(404)
    
    def handle_compile(self):
        content_length = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(content_length).decode('utf-8')
        
        try:
            data = json.loads(body)
            code = data.get('code', '')
            args = data.get('args', [])
        except json.JSONDecodeError as e:
            self.send_json({'error': f'Invalid JSON: {e}'})
            return
        
        if not code.strip():
            self.send_json({'error': 'No code to compile'})
            return
        
        # Create temp file
        with tempfile.NamedTemporaryFile(mode='w', suffix='.loi', delete=False) as f:
            f.write(code)
            temp_file = f.name
        
        try:
            # Build command
            cmd = [LOI_BIN, temp_file] + args
            env = os.environ.copy()
            env['LOI_PATH'] = '/usr/local/share/loi/lib'
            
            # Run with timeout
            proc = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=10,
                env=env
            )
            
            output = proc.stdout
            if proc.stderr:
                output += '\n' + proc.stderr
            
            self.send_json({'output': output})
            
        except subprocess.TimeoutExpired:
            self.send_json({'error': 'Compilation timed out (10s limit)'})
        except Exception as e:
            self.send_json({'error': str(e)})
        finally:
            try:
                os.unlink(temp_file)
            except:
                pass
    
    def send_json(self, data):
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps(data).encode('utf-8'))
    
    def log_message(self, format, *args):
        # Suppress default log messages
        pass

def main():
    os.chdir(SITE_DIR)
    
    with socketserver.TCPServer(("", PORT), PlaygroundHandler) as httpd:
        print(f"Serving loi playground at http://localhost:{PORT}")
        print(f"Site directory: {SITE_DIR}")
        print(f"loi binary: {LOI_BIN}")
        print("Press Ctrl+C to stop")
        try:
            httpd.serve_forever()
        except KeyboardInterrupt:
            print("\nShutting down...")
            httpd.shutdown()

if __name__ == '__main__':
    main()