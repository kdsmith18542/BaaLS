with open('/etc/baals/config.toml', 'r') as f:
    content = f.read()
content = content.replace('level = "info"', 'level = "debug"')
with open('/etc/baals/config.toml', 'w') as f:
    f.write(content)
print('Updated to debug')
