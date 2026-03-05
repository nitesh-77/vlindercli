FROM python:3.12-slim

WORKDIR /app

# Copy and install dependencies first for better layer caching
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copy the application code
COPY todo_server.py .

# Bake /etc/hosts so DNS resolves on read-only filesystems (e.g. Lambda)
COPY hosts /etc/hosts

# Expose the port the app runs on
EXPOSE 8080

# Use gunicorn to run the Flask app
# -w 2: two worker processes
# -b 0.0.0.0:8080: bind to all interfaces on port 8080
# todo_server:app: run the 'app' object from the 'todo_server.py' module
ENTRYPOINT ["gunicorn", "-w", "2", "-b", "0.0.0.0:8080", "todo_server:app"]
