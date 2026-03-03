FROM python:3.12-slim
COPY vlinder-lambda-adapter /opt/extensions/vlinder-lambda-adapter
COPY echo_server.py /echo_server.py
EXPOSE 8080
ENTRYPOINT ["python3", "/echo_server.py"]
