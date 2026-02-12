FROM python:3.12-alpine
WORKDIR /app
COPY vlinder.py .
COPY todo_server.py .
EXPOSE 8080
ENTRYPOINT ["python3", "todo_server.py"]
