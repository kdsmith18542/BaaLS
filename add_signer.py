import urllib.request, json
data = json.dumps({"public_key": "0dcc7432a97fbd69368cc7a865d0e2c42a4a9312b4f06e532e7de33c5ac349b0"}).encode()
req = urllib.request.Request("http://127.0.0.1:18080/api/v1/admin/signers", data=data, headers={"Content-Type": "application/json"}, method="POST")
print(urllib.request.urlopen(req).read().decode())
